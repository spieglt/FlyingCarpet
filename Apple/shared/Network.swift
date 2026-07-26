//
//  Network.swift
//  FlyingCarpet
//
//  Created by Theron on 9/17/22.
//

import Foundation
import Network
import SystemConfiguration

// MARK: - Thread-safe box for Swift 6 concurrency

// Single shared definition used across Network, Discovery, and Transfer.
final class AtomicBool: @unchecked Sendable {
    private var _value: Bool
    private let lock = NSLock()

    init(_ value: Bool) { _value = value }

    var value: Bool {
        get { lock.lock(); defer { lock.unlock() }; return _value }
        set { lock.lock(); defer { lock.unlock() }; _value = newValue }
    }

    /// Atomically sets to true if currently false. Returns true if the swap happened (caller wins the race).
    func testAndSet() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        if !_value { _value = true; return true }
        return false
    }
}

// Dedicated queue for all TCP transfer I/O so connection callbacks don't run on
// the main thread (which would stutter the UI and cap throughput on large files).
let networkQueue = DispatchQueue(label: "dev.flyingcarpet.network")

// MARK: - TCP Connection Protocol

// One protocol with default implementations, so the hotspot client (TCPClient)
// and the shared-network connection wrapper (TCPConnectionWrapper) share the same
// read/write/teardown logic instead of duplicating it.
protocol TCPConnectionProtocol {
    var connection: NWConnection { get }
    func write(data: Data) async throws
    func receiveNBytes(n: Int) async throws -> Data
    func disconnect()
    func forceDisconnect()
}

extension TCPConnectionProtocol {
    func write(data: Data) async throws {
        let error: NWError? = await withCheckedContinuation { continuation in
            self.connection.send(content: data, completion: .contentProcessed { error in
                continuation.resume(returning: error)
            })
        }
        // without this, a send on a dead connection reports success and the sender
        // keeps pumping chunks into the void until a read finally fails
        if let error = error {
            throw error
        }
    }

    func receiveNBytes(n: Int) async throws -> Data {
        var localBuffer = Data()
        var bytesRead = 0

        while bytesRead < n {
            let (content, _, _, error) = await receiveUpToNBytes(n: n - bytesRead)
            if let error = error {
                throw error
            }
            if let content = content {
                localBuffer.append(content)
                bytesRead += content.count
            } else {
                throw TransferError.TCPReadError
            }
        }
        return localBuffer
    }

    func receiveUpToNBytes(n: Int) async -> (Data?, NWConnection.ContentContext?, Bool, NWError?) {
        return await withCheckedContinuation { continuation in
            self.connection.receive(minimumIncompleteLength: 0, maximumLength: n) { content, contentContext, isComplete, error in
                continuation.resume(returning: (content, contentContext, isComplete, error))
            }
        }
    }

    func disconnect() {
        connection.cancel()
    }

    func forceDisconnect() {
        connection.forceCancel()
    }
}

// MARK: - TCP Client (hotspot mode: connect to peer, callback-driven)

class TCPClient: TCPConnectionProtocol {
    var endpoint: NWEndpoint
    var connection: NWConnection

    init(host: String, port: Int,
         teardownCallback: @escaping () -> Void,
         readyCallback: @escaping () -> Void)
    throws {
        if !(port > 0 && port < 65536) {
            throw TransferError.PortError
        }
        let options = NWProtocolTCP.Options()
        options.connectionTimeout = 10 // this controls handshake timeout
        options.connectionDropTime = 10 // let connection drop if it goes quiet for 10 seconds
        // The send loop writes an 8-byte chunk length and then the chunk body, so the length
        // reaches the wire as its own small segment. Nagle holds a sub-MSS segment until the
        // peer ACKs what's already in flight, and a receiver taking a file body has nothing
        // to send back, so that ACK waits out its delayed-ACK timer (200ms on Windows) --
        // one stall per chunk. Measured on the Rust side 2026-07-25 at 38.8mbps where SMB
        // moved the same file between the same two machines at ~600mbps. Every write in this
        // protocol is either already large or one the peer is actively waiting on, so there
        // is nothing for Nagle's coalescing to win.
        options.noDelay = true
        let parameters = NWParameters(tls: nil, tcp: options)
        self.endpoint = NWEndpoint.hostPort(host: NWEndpoint.Host(host), port: NWEndpoint.Port(String(port))!)
        self.connection = NWConnection(to: endpoint, using: parameters)
        // ready and teardown each fire at most once, however the state moves
        // through failed/cancelled afterward
        let becameReady = AtomicBool(false)
        let toreDown = AtomicBool(false)
        self.connection.stateUpdateHandler = { state in
            print("endpoint: \(self.connection.endpoint)")
            switch state {
            case .ready:
                print("connection ready!")
                guard becameReady.testAndSet() else { return }
                readyCallback()
            case .failed(let err):
                print("connection error: \(err)")
                if toreDown.testAndSet() {
                    teardownCallback()
                }
            case .preparing:
                print("preparing connection")
            case .cancelled:
                print("connection cancelled")
                if toreDown.testAndSet() {
                    teardownCallback()
                }
            case .setup:
                print("connection setup")
            case .waiting(let err):
                // the OS keeps retrying the connect while in .waiting, so give it time,
                // but bounded: an unreachable peer otherwise leaves this connection armed
                // forever, and it can fire readyCallback long after the transfer was
                // abandoned (e.g. against the next hotspot at the same gateway IP)
                print("connection waiting: \(err)")
                networkQueue.asyncAfter(deadline: .now() + 30) { [weak self] in
                    if !becameReady.value {
                        self?.connection.cancel()
                    }
                }
            default:
                break
            }
        }
    }
}

func networkToInt64(bytes: Data) -> Int64 {
    if bytes.count < 8 {
        return 0
    }
    // loadUnaligned, not load: Data makes no promise about the alignment of its backing
    // buffer (small values live inline in the struct), and load(fromByteOffset:as:) requires
    // memory aligned for Int64. Every 8-byte protocol field comes through here.
    let bigEndian = bytes.withUnsafeBytes { pointer in
        pointer.loadUnaligned(fromByteOffset: 0, as: Int64.self)
    }
    return Int64(bigEndian: bigEndian)
}

class LocalNetworkPermissionTester {
    var connection: NWConnection
    var success = false
    var semaphore: DispatchSemaphore
    init(semaphore: DispatchSemaphore) {
        self.semaphore = semaphore
        let dispatchQueue = DispatchQueue(label: "LocalNetworkPermissionTester")
        self.connection = NWConnection(host: "127.255.255.255", port: 9, using: .udp)
        self.connection.stateUpdateHandler = { state in
            switch state {
            case .ready:
                self.success = true
                semaphore.signal()
            case .waiting(_):
                if case .localNetworkDenied? = self.connection.currentPath?.unsatisfiedReason {
                    self.success = false
                    semaphore.signal()
                }
            case .failed(_):
                // probe failed for some reason other than a permission denial; don't
                // leave the waiter hanging, and don't claim the permission is missing
                // when we can't tell
                self.success = true
                semaphore.signal()
            default:
                break
            }
        }
        connection.start(queue: dispatchQueue)
    }
}

// MARK: - Network Detection Utilities

// Synchronous getifaddrs lookup, which is fast; it replaced an NWPathMonitor +
// semaphore version that could stall a cooperative thread for up to 2 seconds.
/// Returns the name and IPv4 address of the interface to use for shared network mode.
/// Any connected IPv4 interface works — discovery and TCP are interface-agnostic — so
/// wired devices are supported too. Considers en* interfaces that are up and running:
/// en0 first (WiFi on iPhones and MacBooks, built-in Ethernet on desktop Macs), then
/// the lowest-numbered other en* (Ethernet ports, USB adapters, Thunderbolt docks).
/// The en* filter excludes cellular (pdp_ip*), tunnels (utun*), AWDL, and bridges;
/// link-local (169.254.x.x) addresses are skipped because they mean the interface has
/// no usable network.
func getLocalIPv4Interface() -> (name: String, address: String)? {
    var candidates: [(name: String, address: String)] = []
    var ifaddr: UnsafeMutablePointer<ifaddrs>?

    if getifaddrs(&ifaddr) == 0 {
        var ptr = ifaddr
        while ptr != nil {
            defer { ptr = ptr?.pointee.ifa_next }

            guard let interface = ptr?.pointee else { continue }
            // ifa_addr can be NULL for some interfaces (e.g. certain tunnel/cellular
            // entries); dereferencing it without this guard crashes the app.
            guard let ifaAddr = interface.ifa_addr else { continue }

            // IPv4 only
            guard ifaAddr.pointee.sa_family == UInt8(AF_INET) else { continue }

            // up, running, not loopback
            let flags = interface.ifa_flags
            guard flags & UInt32(IFF_UP) != 0,
                  flags & UInt32(IFF_RUNNING) != 0,
                  flags & UInt32(IFF_LOOPBACK) == 0 else { continue }

            let name = String(cString: interface.ifa_name)
            guard name.hasPrefix("en") else { continue }

            var hostname = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            getnameinfo(
                ifaAddr,
                socklen_t(ifaAddr.pointee.sa_len),
                &hostname,
                socklen_t(hostname.count),
                nil,
                0,
                NI_NUMERICHOST
            )
            let address = String(cString: hostname)
            if !address.isEmpty && !address.hasPrefix("169.254.") {
                candidates.append((name: name, address: address))
            }
        }
        freeifaddrs(ifaddr)
    }

    if let en0 = candidates.first(where: { $0.name == "en0" }) {
        return en0
    }
    return candidates.min { a, b in
        a.name.compare(b.name, options: .numeric) == .orderedAscending
    }
}

// MARK: - TCP Server (for shared network mode receivers)

class TCPServer: @unchecked Sendable {
    private let lock = NSLock()
    private var listener: NWListener?
    private var acceptedConnection: NWConnection?
    private var continuation: CheckedContinuation<NWConnection, Error>?
    private var pendingResult: Result<NWConnection, Error>?  // outcome that arrived before accept() asked for it
    private var finished = false

    /// Bind the TCP listener on the given port. Call this before discovery so the
    /// port is ready when the sender connects. The connection handler is installed
    /// here (before start) so a connection arriving before accept() is called is
    /// stashed rather than dropped.
    func bind(port: UInt16) throws {
        let options = NWProtocolTCP.Options()
        options.connectionTimeout = 10
        options.noDelay = true // see the connect path above: otherwise one Nagle stall per chunk
        let parameters = NWParameters(tls: nil, tcp: options)
        parameters.allowLocalEndpointReuse = true

        guard let nwPort = NWEndpoint.Port(rawValue: port) else {
            throw TransferError.PortError
        }

        let listener = try NWListener(using: parameters, on: nwPort)
        self.listener = listener
        listener.newConnectionHandler = { [weak self] newConnection in
            self?.handleNewConnection(newConnection)
        }
        listener.start(queue: networkQueue)
    }

    private func handleNewConnection(_ newConnection: NWConnection) {
        lock.lock()
        // Hold one candidate connection at a time; reject extras while we have one.
        if acceptedConnection != nil {
            lock.unlock()
            newConnection.cancel()
            return
        }
        acceptedConnection = newConnection
        lock.unlock()

        newConnection.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready:
                // Got a live connection; stop accepting and hand it off.
                self?.listener?.cancel()
                self?.deliver(.success(newConnection))
            case .failed, .cancelled, .waiting:
                // This candidate died before becoming ready (e.g. a stray connection
                // or a sender that dropped). Free the slot and keep listening so the
                // peer's retry can still be accepted, until accept()'s timeout.
                newConnection.cancel()
                self?.releaseAccepted(newConnection)
            default:
                break
            }
        }
        newConnection.start(queue: networkQueue)
    }

    private func releaseAccepted(_ connection: NWConnection) {
        lock.lock()
        if acceptedConnection === connection {
            acceptedConnection = nil
        }
        lock.unlock()
    }

    /// Records an outcome exactly once: resumes a waiting accept(), or stashes it
    /// if the outcome arrived before accept() was called.
    private func deliver(_ result: Result<NWConnection, Error>) {
        lock.lock()
        if finished { lock.unlock(); return }
        if let cont = continuation {
            finished = true
            continuation = nil
            lock.unlock()
            cont.resume(with: result)
        } else {
            if pendingResult == nil { pendingResult = result }  // first outcome wins
            lock.unlock()
        }
    }

    /// Accept one incoming TCP connection. With a nil timeout, waits until a connection
    /// arrives or cancel() is called (the sender may not be started for a long time).
    func accept(timeout: TimeInterval? = nil) async throws -> NWConnection {
        guard listener != nil else {
            throw TransferError.TCPServerError("Listener not bound")
        }
        return try await withCheckedThrowingContinuation { cont in
            lock.lock()
            if finished {
                lock.unlock()
                cont.resume(throwing: TransferError.TCPServerError("Already finished"))
                return
            }
            if let result = pendingResult {
                finished = true
                pendingResult = nil
                lock.unlock()
                cont.resume(with: result)
                return
            }
            continuation = cont
            lock.unlock()

            if let timeout = timeout {
                DispatchQueue.global().asyncAfter(deadline: .now() + timeout) { [weak self] in
                    self?.deliver(.failure(TransferError.TCPServerError("Accept timed out")))
                }
            }
        }
    }

    func cancel() {
        listener?.cancel()
        acceptedConnection?.cancel()
        // Make sure a pending accept() doesn't hang if we're torn down.
        deliver(.failure(TransferError.UserCancelled))
    }
}

// MARK: - TCP connection wrapper for an established NWConnection (shared network mode)

class TCPConnectionWrapper: TCPConnectionProtocol {
    var connection: NWConnection

    init(connection: NWConnection) {
        self.connection = connection
    }

    /// Connect to a peer and return once the connection is ready. Mirrors TCPClient's
    /// connection options (including connectionDropTime) so hotspot and shared-network
    /// connections behave the same on quiet/dropped links.
    static func connect(host: String, port: Int) async throws -> TCPConnectionWrapper {
        let endpoint = NWEndpoint.hostPort(
            host: NWEndpoint.Host(host),
            port: NWEndpoint.Port(rawValue: UInt16(port))!
        )
        let options = NWProtocolTCP.Options()
        options.connectionTimeout = 10
        options.connectionDropTime = 10
        options.noDelay = true // see the connect path above: otherwise one Nagle stall per chunk
        let parameters = NWParameters(tls: nil, tcp: options)
        let connection = NWConnection(to: endpoint, using: parameters)

        let resumed = AtomicBool(false)
        return try await withCheckedThrowingContinuation { continuation in
            connection.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    guard resumed.testAndSet() else { return }
                    continuation.resume(returning: TCPConnectionWrapper(connection: connection))
                case .failed(let error):
                    guard resumed.testAndSet() else { return }
                    continuation.resume(throwing: error)
                case .waiting(let error):
                    // Peer not listening yet; treat as failure so the retry loop tries again.
                    guard resumed.testAndSet() else { return }
                    connection.cancel()
                    continuation.resume(throwing: error)
                case .cancelled:
                    guard resumed.testAndSet() else { return }
                    continuation.resume(throwing: TransferError.UserCancelled)
                default:
                    break
                }
            }
            connection.start(queue: networkQueue)
        }
    }
}
