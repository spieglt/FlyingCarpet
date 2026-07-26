//
//  Transfer.swift
//  FlyingCarpet
//
//  Created by Theron on 6/4/22.
//

import CryptoKit
import Foundation
import Network
import NetworkExtension

// AtomicBool is defined once in Network.swift and shared across the module.

// v10 is a breaking change: shared network mode and its new protocol are not compatible
// with v9 or earlier. See docs/shared-network-crypto.md in the main FlyingCarpet repo.
let VERSION: UInt8 = 10
let PORT = 3290
let CHUNK_SIZE = 5000000 // 5MB
let ONE = Data([0,0,0,0,0,0,0,1])
let ZERO = Data([0,0,0,0,0,0,0,0])

// Sanity bounds on peer-supplied header values, checked before they're used to size a
// loop or an allocation. Chosen so every legitimate transfer passes while negative or
// absurd values (which would trap a range or exhaust memory) are rejected.
let MAX_FILE_COUNT: Int64 = 1_000_000
let MAX_FILENAME_BYTES = 8192              // a path component tree; PATH_MAX is ~1024
let MAX_CHUNK_BYTES = CHUNK_SIZE           // max raw chunk (all platforms use CHUNK_SIZE)

// Shown when two Apple devices try to use hotspot mode (not possible since Apple
// removed programmatic hotspot configuration). Used by both iOS and macOS.
let appleToAppleHotspotErrorMessage = "Error: Flying Carpet's hotspot mode does not work between two Apple devices because Apple no longer lets hotspots be configured programmatically. Use Shared Network mode instead, either over a normal WiFi network or a manually-configured Personal Hotspot joined from the other device."

// MARK: - Connection Mode

enum ConnectionMode {
    case hotspot
    case sharedNetwork
}

// TODO: do more with these?
enum TransferError: Error {
    case NoFilename(msg: String)
    case CouldNotJoinNetwork
    case CouldNotReadFiles
    case CouldNotReadFileSize
    case HostNotIPv4
    case GatewayNotHostPort
    case UserCancelled
    case TCPReadError
    case PortError
    case ModeConflict
    case FileError
    case RandomError
    case IncompatibleVersion
    case NoWifiInterface
    case CouldNotFindSsid
    case TCPServerError(String)
    case UnsafeFilename(name: String)
    case MalformedTransferHeader(String)
}

class Transfer {

    var receiveDir: URL? = nil
    var sendDir: URL? = nil
    var fileList: [URL] = []
    #if os(iOS)
    var sendFolder = false
    #elseif os(macOS)
    var sendFolder = true
    #endif
    var ssid: String = ""
    var password: String = ""
    // Noise PSK (the PBKDF2-stretched password), derived once per transfer: at
    // discovery time in shared network mode (the discovery HMAC key comes from it),
    // or at handshake time in hotspot mode.
    var psk: Data? = nil
    var peerIP = ""
    var gateway: Network.NWEndpoint? = nil
    var task: Task<(), Error>? = nil
    var pathMonitor: NWPathMonitor? = nil
    // Active TCP connection for both modes: TCPClient (hotspot) or TCPConnectionWrapper (shared network).
    var tcp: (any TCPConnectionProtocol)? = nil
    var tcpServer: TCPServer? = nil  // For shared network mode receivers (TCP server)
    var discoveryService: DiscoveryService? = nil  // For shared network mode receivers (background announcer)
    // set on the transfer task, read by killIt() from a separate timeout task
    let confirmed = AtomicBool(false)
    var delegate: Delegate?

    // Shared network mode support
    var connectionMode: ConnectionMode = .hotspot

    enum Mode {
        case Sending
        case Receiving
    }
    var mode: Mode = .Sending
    
    protocol Delegate {
        func output(msg: String)
        func setProgress(_ progress: Float, animated: Bool, hidden: Bool)
        func toggleUI(transferRunning: Bool)
        func forgetHotspot()
        func joinHotspot() async throws
        func emptyDocsDir() throws
        func stopBluetooth()
    }
    
    // Networks we were already on when this transfer started. The wifi path goes on
    // advertising the old default route for a second or two after we associate to the
    // peer's hotspot — DHCP hasn't finished — so awaitPeerGateway() has to be able to
    // tell "the router I was just using" from "the peer". Guarded by gatewayLock: the
    // path monitor delivers on pathQueue, the latch happens on the transfer task.
    private var preJoinGateways: Set<String> = []
    private var preJoinGatewaysLatched = false
    private let gatewayLock = NSLock()

    func startPathMonitor() {
        pathMonitor = NWPathMonitor.init(requiredInterfaceType: .wifi)
        // record every gateway we see until the latch closes, so a path that flaps
        // between networks before the join doesn't leave one of them unrecorded
        pathMonitor?.pathUpdateHandler = { [weak self] path in
            guard let self = self else { return }
            self.gatewayLock.lock()
            defer { self.gatewayLock.unlock() }
            guard !self.preJoinGatewaysLatched else { return }
            self.preJoinGateways.formUnion(ipv4Gateways(in: path))
        }
        pathMonitor?.start(queue: DispatchQueue.init(label: "pathQueue"))
    }

    // Freezes the pre-join gateway set. MUST be called *before* the join is initiated,
    // never after. The version of this that recorded pre-join gateways until a
    // "hotspotJoined" flag flipped after associate()/apply() had returned lost a race:
    // when DHCP on the hotspot landed inside that window, the peer's own gateway was
    // recorded as a pre-join one and then excluded forever, so the transfer hung until
    // the 120-second timeout. Latching up front makes that window not exist.
    func latchPreJoinGateways() {
        gatewayLock.lock()
        defer { gatewayLock.unlock() }
        // idempotent on purpose: macOS retries joinHotspot() after a failed attempt, and
        // by then we may already be associated to the peer. Re-latching there would
        // record the peer's own gateway as a pre-join one and exclude it for the rest of
        // the transfer — the very failure this design exists to prevent.
        guard !preJoinGatewaysLatched else { return }
        preJoinGatewaysLatched = true
        // currentPath can hold a gateway the update handler hasn't delivered yet, and
        // on a fast start it may not have delivered anything at all
        if let path = pathMonitor?.currentPath {
            preJoinGateways.formUnion(ipv4Gateways(in: path))
        }
    }

    // The frozen set, read once at the top of awaitPeerGateway(). Kept synchronous so the
    // lock is taken and released outside the async context (NSLock.lock() is `noasync`).
    private func latchedPreJoinGateways() -> Set<String> {
        gatewayLock.lock()
        defer { gatewayLock.unlock() }
        return preJoinGateways
    }

    // Waits for the peer's hotspot to route us, and records it as `gateway`: we joined
    // as a client, so the peer is this network's gateway. Skips the networks we were on
    // before the join — see latchPreJoinGateways().
    func awaitPeerGateway() async throws {
        self.delegate?.output(msg: "Looking for peer's IP address...")
        let excluded = latchedPreJoinGateways()
        if !excluded.isEmpty {
            print("ignoring pre-join gateways: \(excluded.sorted())")
        }
        var i = 0
        while true {
            await Task.yield()
            if Task.isCancelled {
                throw TransferError.UserCancelled
            }
            // we're associated to the hotspot's SSID, so the wifi path's gateway
            // is the peer's IP; it shows up once DHCP completes.
            if let path = self.pathMonitor?.currentPath,
               let gateway = firstIPv4Gateway(in: path, excluding: excluded) {
                self.gateway = gateway
                self.delegate?.output(msg: "Peer IP: \(gateway)")
                return
            }
            if i == 120 {
                throw TransferError.CouldNotJoinNetwork
            }
            if i > 0 && i % 10 == 0 {
                self.delegate?.output(msg: "Still looking for peer IP...")
            }
            i += 1
            // Task.sleep, not sleep(): this runs on a cooperative-pool thread
            try? await Task.sleep(nanoseconds: 1_000_000_000)
        }
    }

    func handleFileSelection(urls: [URL]) throws {
        // get access to files/folders in Files app
        for url in urls {
            print("url: \(url)")
            guard url.startAccessingSecurityScopedResource() else {
                throw TransferError.CouldNotReadFiles
            }
        }
        if self.mode == .Sending {
            #if os(iOS)
            if self.sendFolder {
                print("getting files")
                let files = try getFilesInDir(url: urls[0])
                self.fileList = files
                // strip the *parent* of the chosen folder, not the folder itself, so the
                // folder's own name leads every relative path and the receiving device
                // recreates it with the files inside. Matches macOS below, and the desktop
                // and Android senders; see docs/send-folder-behavior.md in the Rust repo.
                self.sendDir = urls[0].deletingLastPathComponent()
                // print("got files: \(files)")
            } else {
                self.fileList = urls
                print("fileList: \(self.fileList)")
            }
            #elseif os(macOS)
            self.sendDir = urls[0].deletingLastPathComponent()
            for url in urls {
                let isDir = (try url.resourceValues(forKeys: [.isDirectoryKey])).isDirectory ?? false
                if isDir {
                    let files = try getFilesInDir(url: url)
                    self.fileList.append(contentsOf: files)
                } else {
                    self.fileList.append(url)
                }
            }
            #endif
        } else {
            // should have the folder the user chose
            self.receiveDir = urls[0]
        }
    }

    func confirmVersion() async throws {
        guard let tcp = self.tcp else { throw TransferError.TCPReadError }

        // both sides send version, then read peer's version (symmetric, works via TCP buffering)
        let versionBytes = Data([0, 0, 0, 0, 0, 0, 0, VERSION])
        try await tcp.write(data: versionBytes)
        // receive peer's version
        let peerVersionBytes = try await tcp.receiveNBytes(n: 8)
        let peerVersion = networkToInt64(bytes: peerVersionBytes)
        if peerVersion < VERSION { // we make decision
            if isCompatible(peerVersion: peerVersion) {
                try await tcp.write(data: ONE)
            } else {
                try await tcp.write(data: ZERO)
                self.delegate?.output(msg: "The other device is running Flying Carpet version \(peerVersion), which is not compatible with this version (\(VERSION)). Please update both devices to the latest version at https://flyingcarpet.spiegl.dev.")
                throw TransferError.IncompatibleVersion
            }
        } else if peerVersion > VERSION { // peer makes decision
            let confirmationBytes = try await tcp.receiveNBytes(n: 8)
            let confirmed = networkToInt64(bytes: confirmationBytes)
            if confirmed == 0 {
                self.delegate?.output(msg: "The other device is running Flying Carpet version \(peerVersion), which is not compatible with this version (\(VERSION)). Please update both devices to the latest version at https://flyingcarpet.spiegl.dev.")
                throw TransferError.IncompatibleVersion
            }
        }
        // if versions matched, they're compatible
    }

    func confirmMode() async throws {
        guard let tcp = self.tcp else { throw TransferError.TCPReadError }

        let myMode: Data = (self.mode == .Sending) ? ONE : ZERO  // 1 = sending, 0 = receiving

        switch connectionMode {
        case .sharedNetwork:
            // Symmetric: both sides send their mode, read the peer's, verify they're opposite.
            // Matches the Rust/Android SharedNetwork path.
            try await tcp.write(data: myMode)
            let peerModeBytes = try await tcp.receiveNBytes(n: 8)
            let peerMode = networkToInt64(bytes: peerModeBytes)
            let myModeValue: Int64 = (self.mode == .Sending) ? 1 : 0
            if peerMode == myModeValue {
                throw TransferError.ModeConflict
            }
        case .hotspot:
            // Asymmetric, for backward compatibility. Apple devices are always the
            // hotspot guest, so we tell the host our mode and wait for it to confirm
            // (1 = ok, anything else = both sides picked the same mode). Matches the
            // Rust WifiClient branch in confirm_mode().
            try await tcp.write(data: myMode)
            let responseBytes = try await tcp.receiveNBytes(n: 8)
            let response = networkToInt64(bytes: responseBytes)
            if response != 1 {
                throw TransferError.ModeConflict
            }
        }
    }

    func sendAndReceive() {
        let transferTask = Task.init(priority: .high) {
            defer {
                self.cleanUpTransfer()
            }
            // Plaintext preamble on the raw connection: version, then send/receive mode.
            // Every preamble byte, sent and received, is recorded and bound into the Noise
            // prologue below, so tampering with the preamble fails the handshake.
            guard let rawTcp = self.tcp else {
                self.delegate?.output(msg: "Error: no TCP connection at transfer start.")
                return
            }
            let preamble = RecordingTCPConnection(inner: rawTcp)
            self.tcp = preamble
            do {
                try await self.confirmVersion()
            } catch TransferError.IncompatibleVersion {
                // confirmVersion() already emitted a specific, user-facing message
                // naming both versions before throwing.
                return
            }
            // confirm mode
            do {
                try await self.confirmMode()
            } catch TransferError.ModeConflict {
                self.delegate?.output(msg: "Error: Both sides picked the same mode. One side must select Send and the other Receive.")
                return
            } catch {
                self.delegate?.output(msg: "Error confirming mode: \(error)")
                return
            }
            self.tcp = preamble.inner
            // Establish the Noise encrypted transport over the same connection, for both
            // modes, with the preamble transcript bound in as the prologue. Everything
            // after this — file count, metadata, and file data — is confidential and
            // tamper-evident. The Noise initiator is the TCP client: the shared-network
            // sender, or the hotspot guest (Apple always joins a hotspot, it cannot host
            // one). A wrong password (or a tampered preamble) fails the handshake with a
            // clear message.
            do {
                let role: NoiseRole
                if self.connectionMode == .sharedNetwork {
                    role = (self.mode == .Sending) ? .initiator : .responder
                } else {
                    role = .initiator
                }
                let prologue = role == .initiator
                    ? buildPrologue(initiatorTranscript: preamble.sent, responderTranscript: preamble.received)
                    : buildPrologue(initiatorTranscript: preamble.received, responderTranscript: preamble.sent)
                self.delegate?.output(msg: "Establishing encrypted connection...")
                // in shared network mode the PSK was already derived to key discovery;
                // hotspot mode derives it here (one PBKDF2 run per transfer either way)
                let psk = self.psk ?? derivePsk(self.password)
                self.tcp = try await noiseHandshake(tcp: preamble.inner, role: role, psk: psk, prologue: prologue)
                self.delegate?.output(msg: "Encrypted connection established.")
            } catch {
                self.delegate?.output(msg: "\(error)")
                return
            }
            // send or receive the files
            do {
                if self.mode == .Sending {
                    try await self.sendFiles()
                } else {
                    try await self.receiveFiles()
                }
            } catch {
                self.delegate?.output(msg: "Transfer error: \(error)")
            }
        }
        // store it (on main, where cleanUpTransfer reads it) so cancelling cancels the
        // transfer loop itself, not just the setup task that spawned it
        DispatchQueue.main.async {
            self.task = transferTask
        }
    }
    
    func cleanUpTransfer() {
        // self.delegate?.output(msg: "Cleaning up transfer")
        print("Cleaning up transfer")
        DispatchQueue.main.async {
            // cancel transfer
            self.task?.cancel()
            // close tcp connection (both modes)
            self.tcp?.disconnect()
            self.tcp = nil
            self.tcpServer?.cancel()
            self.tcpServer = nil
            // stop shared network discovery announcements if they're running
            self.discoveryService?.cancel()
            self.discoveryService = nil
            // leave hotspot (only in hotspot mode)
            if self.connectionMode == .hotspot {
                self.delegate?.forgetHotspot()
            }
            // stop the path monitor
            self.pathMonitor?.cancel()
            self.pathMonitor = nil
            // forget gateway
            self.gateway = nil
            // reset peer IP
            self.peerIP = ""
            // forget the derived PSK (the next transfer has a new password)
            self.psk = nil
            // close open files
            for url in self.fileList {
                url.stopAccessingSecurityScopedResource()
            }
            self.receiveDir?.stopAccessingSecurityScopedResource()
            // clear out temp docs copied from photo roll
            do {
                try self.delegate?.emptyDocsDir()
            } catch {
                self.delegate?.output(msg: "Error emptying temporary camera roll contents from app's documents directory: \(error).")
            }
            // toggle UI
            self.delegate?.toggleUI(transferRunning: false)

            // shut down bluetooth
            self.delegate?.stopBluetooth()
        }
    }
    
    func runTransfer() async {
        switch connectionMode {
        case .hotspot:
            await runHotspotTransfer()
        case .sharedNetwork:
            await runSharedNetworkTransfer()
        }
    }

    // MARK: - Hotspot Mode Transfer (original behavior)

    func runHotspotTransfer() async {
        startPathMonitor()

        // get ssid (the SHA256-derived key is only used for the SSID digits now; the
        // transfer itself is encrypted by the Noise handshake keyed from the PSK)
        do {
            let (ssid, _) = try getSsidAndKey(password: password)
            // if connecting to Android, SSID would've been set when scanning to QR code
            if self.ssid == "" {
                self.ssid = ssid
            }
        } catch {
            self.delegate?.output(msg: "Error getting SSID: \(error)")
            self.cleanUpTransfer()
            return
        }

        // join hotspot
        while true {
            do {
                print("joining \(self.ssid)")
                try await self.delegate?.joinHotspot()
                break
            } catch {
#if os(iOS)
                if let error = error as NSError? {
                    if error.domain == "NEHotspotConfigurationErrorDomain" {
                        if error.code == NEHotspotConfigurationError.userDenied.rawValue {
                            self.delegate?.output(msg: "User cancelled joining other device's network.")
                        } else if error.code == NEHotspotConfigurationError.alreadyAssociated.rawValue {
                            self.delegate?.output(msg: "Error: device was already connected to hotspot. Disconnecting, please try transfer again.")
                        } else {
                            self.delegate?.output(msg: "Error joining hotspot: \(error)")
                        }
                    } else {
                        self.delegate?.output(msg: "Error joining hotspot: \(error)")
                    }
                }
                self.cleanUpTransfer()
                self.delegate?.output(msg: "Exiting transfer.")
                return
#elseif os(macOS)
                if Task.isCancelled {
                    self.cleanUpTransfer()
                    self.delegate?.output(msg: "Exiting transfer.")
                    return
                } else {
                    print("Could not join hotspot: \(error)")
                    self.delegate?.output(msg: "Looking for SSID \(self.ssid)...")
                    // Task.sleep, not sleep(): this runs on a cooperative-pool thread
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    continue
                }
#endif
            }
        }

        if Task.isCancelled {
            self.delegate?.output(msg: "Cancelled before send/receive, exiting.")
            self.cleanUpTransfer()
            return
        }

        // because iOS will always be ad hoc network guest, the peer should always be the gateway.
        // joinHotspot() found the gateway before returning.
        let ip: String
        do {
            guard let gateway = self.gateway else {
                throw TransferError.CouldNotJoinNetwork
            }
            ip = try gatewayToIPString(gateway: gateway)
        } catch {
            self.delegate?.output(msg: "Error getting gateway IP: \(error)")
            self.cleanUpTransfer()
            return
        }

        // make tcp connection
        let client: TCPClient
        do {
            // TODO: cleanUpTransfer() starts with a deferred call to cleanUpTransfer, then we have it as the teardownCallback here. is that why we're double printing?
            client = try TCPClient(
                host: ip,
                port: PORT,
                teardownCallback: self.cleanUpTransfer,
                readyCallback: self.sendAndReceive
            )
        } catch {
            self.delegate?.output(msg: "Could not construct TCP client: \(error)")
            self.cleanUpTransfer()
            return
        }
        self.tcp = client
        print("starting connection")
        client.connection.start(queue: networkQueue)
        return
    }

    // MARK: - Shared Network Mode Transfer

    func runSharedNetworkTransfer() async {
        // 1. Find a usable network interface. WiFi or wired both work: discovery and
        // TCP are interface-agnostic, so a wired Mac can transfer with a wireless peer.
        guard let localInterface = getLocalIPv4Interface() else {
            self.delegate?.output(msg: "No network connection. Connect to a network (WiFi or wired) or use Hotspot mode.")
            self.cleanUpTransfer()
            return
        }
        let localIP = localInterface.address
        self.delegate?.output(msg: "Local IP: \(localIP) (\(localInterface.name))")

        // 2. Derive the discovery key from the password. The Noise PSK is derived first
        // (one PBKDF2 run — the handshake needs it anyway) and the discovery HMAC key
        // from it, so a captured announcement costs an offline attacker 600k PBKDF2
        // iterations per password guess: no fast hash of the password ever goes on the
        // air. See deriveDiscoveryKey in Noise.swift.
        if password.isEmpty {
            self.delegate?.output(msg: "Error: No password set for shared network discovery.")
            self.cleanUpTransfer()
            return
        }
        let psk = derivePsk(password)
        self.psk = psk
        let keyBytes = [UInt8](deriveDiscoveryKey(psk: psk))

        // 3. Create discovery service
        let role: DiscoveryRole = (mode == .Sending) ? .sender : .receiver
        let discovery = DiscoveryService(
            key: keyBytes,
            role: role,
            localIP: localIP,
            output: { [weak self] msg in self?.delegate?.output(msg: msg) }
        )

        // 4. Receiver is TCP server (consistent with hotspot same-platform convention
        // where the receiver hosts). Bind listener *before* discovery so it's ready
        // when the sender connects immediately after discovering us.
        if mode == .Receiving {
            do {
                let server = TCPServer()
                try server.bind(port: UInt16(PORT))
                self.tcpServer = server
                self.delegate?.output(msg: "TCP listener ready on port \(PORT).")
            } catch {
                self.delegate?.output(msg: "Error starting TCP server: \(error)")
                self.cleanUpTransfer()
                return
            }
        }

        // 5/6. Discover peer and establish the TCP connection based on role.
        // In shared network mode: Receivers listen (TCP server), Senders connect (TCP client)
        if mode == .Receiving {
            // The sender discovers us and connects, and it stops announcing as soon as
            // it hears us — possibly before we ever hear it. So the TCP connection
            // itself is the receiver's completion signal: discovery runs in the
            // background only to announce our presence and surface diagnostics
            // (receiver-role discoverPeer() never returns a peer), and must not gate
            // the accept. No timeout on the accept either: the sender may not be
            // started for a long time.
            guard let server = self.tcpServer else {
                self.delegate?.output(msg: "Error: TCP server not available.")
                self.cleanUpTransfer()
                return
            }
            self.delegate?.output(msg: "Searching for the sender and waiting for it to connect...")
            self.discoveryService = discovery
            let discoveryTask = Task { try? await discovery.discoverPeer() }
            do {
                let connection = try await server.accept()
                discovery.cancel()
                discoveryTask.cancel()
                self.discoveryService = nil
                self.tcp = TCPConnectionWrapper(connection: connection)
                self.delegate?.output(msg: "TCP connection accepted")
            } catch {
                discovery.cancel()
                discoveryTask.cancel()
                self.discoveryService = nil
                self.delegate?.output(msg: "Error accepting TCP connection: \(error)")
                self.cleanUpTransfer()
                return
            }
        } else {
            // Sender: discover the receiver, then connect to it.
            self.delegate?.output(msg: "Searching for peer on network...")
            let peerIP: String
            do {
                peerIP = try await discovery.discoverPeer()
                self.peerIP = peerIP
                self.delegate?.output(msg: "Found peer at \(peerIP)")
            } catch DiscoveryError.cancelled {
                self.delegate?.output(msg: "Discovery cancelled.")
                self.cleanUpTransfer()
                return
            } catch {
                self.delegate?.output(msg: "Discovery failed: \(error)")
                self.cleanUpTransfer()
                return
            }

            if Task.isCancelled {
                self.delegate?.output(msg: "Cancelled, exiting.")
                self.cleanUpTransfer()
                return
            }

            // Connect to receiver
            self.delegate?.output(msg: "Connecting to receiver at \(peerIP):\(PORT)...")

            // Retry connection for up to 30 seconds.
            // The receiver may still be finishing discovery when we start connecting.
            var connected = false
            let connectDeadline = Date().addingTimeInterval(30)
            var attempt = 0
            while Date() < connectDeadline {
                attempt += 1
                if Task.isCancelled {
                    self.delegate?.output(msg: "Cancelled, exiting.")
                    self.cleanUpTransfer()
                    return
                }

                do {
                    self.tcp = try await TCPConnectionWrapper.connect(host: peerIP, port: PORT)
                    connected = true
                    break
                } catch {
                    if Date() < connectDeadline {
                        self.delegate?.output(msg: "Connection attempt \(attempt) failed, retrying...")
                        try? await Task.sleep(nanoseconds: 2_000_000_000)
                    } else {
                        self.delegate?.output(msg: "Failed to connect to peer after \(attempt) attempts: \(error)")
                    }
                }
            }

            if !connected {
                self.cleanUpTransfer()
                return
            }
            self.delegate?.output(msg: "TCP connection established")
        }

        // 6. Proceed with version/mode confirmation and transfer
        sendAndReceive()
    }

    // used to time out reception of final confirmation in case sending end tears things down before we hear it
    func killIt() {
        if !confirmed.value {
            self.tcp?.forceDisconnect()
        }
    }
}

// The first usable IPv4 gateway on the path, or nil if none has appeared yet. A hotspot
// can be dual-stack; the peer's TCP server is IPv4 (SSID, DHCP, and Windows's fixed
// 192.168.137.1 host are all IPv4), so an IPv6 gateway arriving first must be skipped —
// not grabbed and then rejected by gatewayToIPString.
//
// `excluding` holds the gateway IPs we were on *before* joining the peer's hotspot. The
// wifi path keeps reporting the previous network's default route for a second or two
// after association completes (DHCP on the hotspot hasn't finished yet), so without this
// the very first poll hands back the router we were just on and the whole transfer then
// tries to reach the peer at, say, the home router's 192.168.86.1:3290 and times out.
func firstIPv4Gateway(in path: Network.NWPath, excluding excluded: Set<String> = []) -> Network.NWEndpoint? {
    path.gateways.first { gateway in
        guard case .hostPort(let host, _) = gateway, case .ipv4(let ip) = host else { return false }
        return !excluded.contains("\(ip)")
    }
}

// Every IPv4 gateway on the path, as strings — a path can carry more than one.
func ipv4Gateways(in path: Network.NWPath) -> [String] {
    path.gateways.compactMap { gateway in
        guard case .hostPort(let host, _) = gateway, case .ipv4(let ip) = host else { return nil }
        return "\(ip)"
    }
}

func gatewayToIPString(gateway: Network.NWEndpoint) throws -> String {
    var ip: IPv4Address
    switch gateway {
    case .hostPort(let host, _):
        switch host {
        case let .ipv4(_ip):
            ip = _ip
        default:
            throw TransferError.HostNotIPv4
        }
    default:
        throw TransferError.GatewayNotHostPort
    }
    return String("\(ip)")
}

func makeHumanReadableFileSize(size: Int) -> String {
    let n = Float(size)
    switch n {
    case 0 ..< 1_000:
        return String(format: "%.0f bytes", n)
    case 0 ..< 1_000_000:
        return String(format: "%.2fKB", n / 1_000)
    case 0 ..< 1_000_000_000:
        return String(format: "%.2fMB", n / 1_000_000)
    default:
        return String(format: "%.2fGB", n / 1_000_000_000)
    }
}

func formatTime(seconds: Double) -> String {
    if seconds > 60 {
        let minutes = Int(seconds) / 60
        let seconds = seconds.truncatingRemainder(dividingBy: 60)
        return String(format: "%d minutes %.2f seconds", minutes, seconds)
    } else {
        return String(format: "%.2f seconds", seconds)
    }
}

func isCompatible(peerVersion: Int64) -> Bool {
    // v10 (shared network mode and the new protocol) is a clean break from earlier
    // versions. If transferring with a higher version, that version decides compatibility.
    return peerVersion >= 10
}

func getFilesInDir(url: URL) throws -> [URL] {
    var fileURLs: [URL] = []

    var error: NSError? = nil
    NSFileCoordinator().coordinate(readingItemAt: url, error: &error) { (coordinated_url) in

        let resourceKeys: [URLResourceKey] = [.isDirectoryKey, .nameKey, .pathKey]
        let resourceKeysSet = Set<URLResourceKey>(resourceKeys)

        let enumerator = FileManager.default.enumerator(at: coordinated_url, includingPropertiesForKeys: resourceKeys)
        for case let file as URL in enumerator! {
            var resourceValues: URLResourceValues
            do {
                resourceValues = try file.resourceValues(forKeys: resourceKeysSet)
            } catch {
                return
            }
            guard let isDir = resourceValues.isDirectory else {
                continue
            }
            if !isDir {
                // add it to the list
                fileURLs.append(file)
                print("file: \(file)")
            }
        }

    }

    return fileURLs
}

/// Resolves a peer-supplied filename against baseDir, guaranteeing the result stays
/// inside baseDir. Folder transfers legitimately carry subdirectories (e.g.
/// "photos/2024/a.jpg"), so "/" separators are allowed, but path components that would
/// climb out are not.
///
/// The filename arrives as a raw UTF-8 string (not percent-encoded), so there is no
/// encoding layer to peel and no blocklist to bypass: we split on "/", reject any ".."
/// component, and drop empty/"." components (which collapses "a//b" and makes an
/// absolute-looking "/x" land *inside* baseDir rather than at the filesystem root).
/// As a backstop that doesn't depend on the component checks being exhaustive, the
/// lexically-standardized result must still be contained in baseDir.
func safeDestinationURL(baseDir: URL, filename: String) throws -> URL {
    // Canonicalize the base once and build the destination from it, so the containment
    // check below compares like with like. standardizedFileURL resolves the /private/var
    // (and /private/tmp) firmlinks, but only for a path that exists on disk: the base
    // directory collapses to /var/... while a freshly appended, not-yet-created file
    // stays /private/var/..., so a safe bare filename would otherwise fail the prefix
    // check. iOS document-picker folders arrive in /private/var/mobile/... form, so this
    // hit essentially every received file.
    let canonicalBase = baseDir.standardizedFileURL
    var dest = canonicalBase
    var appendedCount = 0
    for component in filename.split(separator: "/", omittingEmptySubsequences: false) {
        if component.isEmpty || component == "." {
            continue
        }
        if component == ".." {
            throw TransferError.UnsafeFilename(name: filename)
        }
        dest.append(path: String(component))
        appendedCount += 1
    }
    guard appendedCount > 0 else {
        throw TransferError.UnsafeFilename(name: filename)
    }
    let base = canonicalBase.path
    let resolved = dest.standardizedFileURL.path
    let basePrefix = base.hasSuffix("/") ? base : base + "/"
    guard resolved == base || resolved.hasPrefix(basePrefix) else {
        throw TransferError.UnsafeFilename(name: filename)
    }
    return dest
}

func makeParentDirectories(for fileURL: URL) throws {
    let dir = fileURL.deletingLastPathComponent()
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
}

// Hashes the whole file regardless of where the handle is sitting, and leaves the offset
// where it found it. The sending end hands us the same handle it's about to read the file
// body from (see checkForFileSending), so consuming the handle here would leave that read
// at EOF. Rust's hash_file and Kotlin's hashFile sidestep this by opening the file
// themselves; restoring the offset keeps this port equivalent.
func hashFile(file: FileHandle) throws -> SHA256Digest {
    let originalOffset = try file.offset()
    defer { try? file.seek(toOffset: originalOffset) }
    try file.seek(toOffset: 0)
    var hasher = SHA256()
    while let data = try file.read(upToCount: 10_000_000), !data.isEmpty {
        hasher.update(data: data)
    }
    return hasher.finalize()
}

func getFileSize(file: URL) throws -> Int {
    let fileAttributes = try FileManager.default.attributesOfItem(atPath: file.path)
    let fileSize: Int
    if let s: Int = fileAttributes[.size] as? Int {
        fileSize = s
    } else {
        throw TransferError.CouldNotReadFileSize
    }
    return fileSize
}

func intToBigEndianBytes(n: Int) -> Data {
    return Data(withUnsafeBytes(of: Int64(bigEndian: Int64(n)), Array.init))
}

func getSsidAndKey(password: String) throws -> (String, SymmetricKey) {
    let bytes = Data(password.utf8)
    let sha256Hash = SHA256.hash(data: bytes)
    let keyBytes: [UInt8] = Array(sha256Hash.makeIterator())
    let ssid = String(format: "flyingCarpet_%02x%02x", keyBytes[0], keyBytes[1])
    let key = SymmetricKey(data: keyBytes)
    return (ssid, key)
}

func randomString(length: Int) -> String {
  let letters = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
  return String((0..<length).map{ _ in letters.randomElement()! })
}

// Same charset and length as the desktop version's generate_password() and Android's
// generatePassword(): 57 confusables-free symbols (no 0/O, 1/l/I — the receiver displays
// the password and the sender types it). 10 chars ≈ 2^58, so a precomputed PBKDF2 table
// over the whole password space (possible because the PSK salt is a fixed domain string)
// is infeasible. SystemRandomNumberGenerator is cryptographically secure on Apple
// platforms.
func generatePassword() -> String {
    let chars = "23456789abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ"
    return String((0..<10).map { _ in chars.randomElement()! })
}


// TODO MACOS:

// TODO IOS:
// how to register to be able to launch flying carpet from other apps? - action extension? share extension?
// why slow to send to but fast to receive from android?

// TODO BOTH:

// FINALLY:
// error checking: check try, ?, !

// MAYBE LATER:
// iOS use https://apple.github.io/swift-nio/docs/current/NIOCore/Structs/CircularBuffer.html instead of normal Data array?
// macOS let user pair manually? how to get current wifi creds to tell peer? don't need to if already paired, just need to see if :3290 is open on gateway and start transfer if so? how to get gateway on iOS if routes didn't change?
// macOS replace location services permissions with shelling out to join wifi?
// macOS camera permission check?
// macOS drag and drop to start transfer? print error about selecting mode first if it's not done?
// resize NSSegmentedControls when resizing window?
// test on asahi

// COMPLAINTS AT APPLE:
// let devs programmatically start hotspots
// scanning for ssids should not require location services. bad way to paper over leaving your wifi database open to the world.
// macos drops connections when acting as peripheral, paired with android and linux. and does not list services after pairing with windows. (solved android by not preventing legacy pairing?)
// can't programmatically unpair bluetooth, apparently a security measure
