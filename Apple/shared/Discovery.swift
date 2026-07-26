//
//  Discovery.swift
//  FlyingCarpet
//
//  Created for shared network peer discovery.
//

import Darwin
import Foundation
import CryptoKit

// AtomicBool is defined once in Network.swift and shared across the module.

// MARK: - Constants

let MULTICAST_ADDR = "239.255.73.67"
let DISCOVERY_PORT: UInt16 = 3290
let DISCOVERY_MAGIC: [UInt8] = [0x46, 0x43, 0x41, 0x50]  // "FCAP"
let ANNOUNCEMENT_SIZE = 93  // 4 + 2 + 1 + 4 + 4 + 2 + 8 + 4 + 32 + 32
let TIMESTAMP_WINDOW_SECS: UInt64 = 60
let DISCOVERY_INTERVAL_MS: UInt64 = 500

// MARK: - Discovery Role

enum DiscoveryRole: UInt8 {
    case sender = 0
    case receiver = 1

    var opposite: DiscoveryRole {
        switch self {
        case .sender: return .receiver
        case .receiver: return .sender
        }
    }
}

// MARK: - Discovery Errors

enum DiscoveryError: Error {
    case cancelled
    case networkError(String)
    case hmacVerificationFailed
    case invalidAnnouncement
    case noLocalIP
}

// MARK: - Discovery Announcement

struct DiscoveryAnnouncement {
    var magic: [UInt8]           // 4 bytes - "FCAP"
    var version: UInt16          // 2 bytes
    var role: DiscoveryRole      // 1 byte
    var capabilities: UInt32     // 4 bytes
    var ipAddress: [UInt8]       // 4 bytes (IPv4)
    var port: UInt16             // 2 bytes
    var timestamp: UInt64        // 8 bytes
    var sequence: UInt32         // 4 bytes
    var nonce: [UInt8]           // 32 bytes
    var hmac: [UInt8]            // 32 bytes

    init(role: DiscoveryRole, ipAddress: String, sequence: UInt32) {
        self.magic = DISCOVERY_MAGIC
        self.version = 1
        self.role = role
        self.capabilities = 0
        self.ipAddress = DiscoveryAnnouncement.ipStringToBytes(ipAddress)
        self.port = DISCOVERY_PORT
        self.timestamp = UInt64(Date().timeIntervalSince1970)
        self.sequence = sequence
        self.nonce = DiscoveryAnnouncement.generateNonce()
        self.hmac = [UInt8](repeating: 0, count: 32)
    }

    private static func ipStringToBytes(_ ip: String) -> [UInt8] {
        let components = ip.split(separator: ".").compactMap { UInt8($0) }
        guard components.count == 4 else {
            return [0, 0, 0, 0]
        }
        return components
    }

    private static func generateNonce() -> [UInt8] {
        var bytes = [UInt8](repeating: 0, count: 32)
        _ = SecRandomCopyBytes(kSecRandomDefault, 32, &bytes)
        return bytes
    }

    func getIPString() -> String {
        return "\(ipAddress[0]).\(ipAddress[1]).\(ipAddress[2]).\(ipAddress[3])"
    }

    // MARK: - Serialization

    func serialize() -> Data {
        var data = Data(capacity: ANNOUNCEMENT_SIZE)

        // magic (4 bytes)
        data.append(contentsOf: magic)

        // version (2 bytes, big-endian)
        data.append(contentsOf: withUnsafeBytes(of: version.bigEndian) { Array($0) })

        // role (1 byte)
        data.append(role.rawValue)

        // capabilities (4 bytes, big-endian)
        data.append(contentsOf: withUnsafeBytes(of: capabilities.bigEndian) { Array($0) })

        // ipAddress (4 bytes)
        data.append(contentsOf: ipAddress)

        // port (2 bytes, big-endian)
        data.append(contentsOf: withUnsafeBytes(of: port.bigEndian) { Array($0) })

        // timestamp (8 bytes, big-endian)
        data.append(contentsOf: withUnsafeBytes(of: timestamp.bigEndian) { Array($0) })

        // sequence (4 bytes, big-endian)
        data.append(contentsOf: withUnsafeBytes(of: sequence.bigEndian) { Array($0) })

        // nonce (32 bytes)
        data.append(contentsOf: nonce)

        // hmac (32 bytes)
        data.append(contentsOf: hmac)

        return data
    }

    static func deserialize(_ data: Data) -> DiscoveryAnnouncement? {
        guard data.count >= ANNOUNCEMENT_SIZE else { return nil }

        let bytes = Array(data)
        var offset = 0

        // magic (4 bytes)
        let magic = Array(bytes[offset..<offset+4])
        guard magic == DISCOVERY_MAGIC else { return nil }
        offset += 4

        // version (2 bytes, big-endian)
        let version = UInt16(bytes[offset]) << 8 | UInt16(bytes[offset+1])
        offset += 2

        // role (1 byte)
        guard let role = DiscoveryRole(rawValue: bytes[offset]) else { return nil }
        offset += 1

        // capabilities (4 bytes, big-endian)
        let capabilities = UInt32(bytes[offset]) << 24 | UInt32(bytes[offset+1]) << 16 |
                          UInt32(bytes[offset+2]) << 8 | UInt32(bytes[offset+3])
        offset += 4

        // ipAddress (4 bytes)
        let ipAddress = Array(bytes[offset..<offset+4])
        offset += 4

        // port (2 bytes, big-endian)
        let port = UInt16(bytes[offset]) << 8 | UInt16(bytes[offset+1])
        offset += 2

        // timestamp (8 bytes, big-endian)
        let timestamp = UInt64(bytes[offset]) << 56 | UInt64(bytes[offset+1]) << 48 |
                       UInt64(bytes[offset+2]) << 40 | UInt64(bytes[offset+3]) << 32 |
                       UInt64(bytes[offset+4]) << 24 | UInt64(bytes[offset+5]) << 16 |
                       UInt64(bytes[offset+6]) << 8 | UInt64(bytes[offset+7])
        offset += 8

        // sequence (4 bytes, big-endian)
        let sequence = UInt32(bytes[offset]) << 24 | UInt32(bytes[offset+1]) << 16 |
                      UInt32(bytes[offset+2]) << 8 | UInt32(bytes[offset+3])
        offset += 4

        // nonce (32 bytes)
        let nonce = Array(bytes[offset..<offset+32])
        offset += 32

        // hmac (32 bytes)
        let hmac = Array(bytes[offset..<offset+32])

        var announcement = DiscoveryAnnouncement(
            role: role,
            ipAddress: "\(ipAddress[0]).\(ipAddress[1]).\(ipAddress[2]).\(ipAddress[3])",
            sequence: sequence
        )
        announcement.magic = magic
        announcement.version = version
        announcement.capabilities = capabilities
        announcement.port = port
        announcement.timestamp = timestamp
        announcement.nonce = nonce
        announcement.hmac = hmac

        return announcement
    }

    // MARK: - HMAC

    mutating func sign(key: [UInt8]) {
        let data = serialize()
        let dataWithoutHmac = data.prefix(ANNOUNCEMENT_SIZE - 32)
        self.hmac = computeHMAC(key: key, data: Data(dataWithoutHmac))
    }

    func verify(key: [UInt8]) -> Bool {
        let data = serialize()
        let dataWithoutHmac = data.prefix(ANNOUNCEMENT_SIZE - 32)
        return verifyHMAC(key: key, data: Data(dataWithoutHmac), expected: hmac)
    }

    func isTimestampValid() -> Bool {
        let now = UInt64(Date().timeIntervalSince1970)
        if timestamp > now {
            return timestamp - now <= TIMESTAMP_WINDOW_SECS
        } else {
            return now - timestamp <= TIMESTAMP_WINDOW_SECS
        }
    }
}

// MARK: - HMAC Utilities

func computeHMAC(key: [UInt8], data: Data) -> [UInt8] {
    let symmetricKey = SymmetricKey(data: Data(key))
    let signature = HMAC<SHA256>.authenticationCode(for: data, using: symmetricKey)
    return Array(signature)
}

func verifyHMAC(key: [UInt8], data: Data, expected: [UInt8]) -> Bool {
    let computed = computeHMAC(key: key, data: data)
    guard computed.count == expected.count else { return false }
    // Constant-time comparison
    var result: UInt8 = 0
    for i in 0..<computed.count {
        result |= computed[i] ^ expected[i]
    }
    return result == 0
}

// MARK: - Discovery Service

class DiscoveryService {
    let key: [UInt8]
    let role: DiscoveryRole
    let localIP: String
    let output: (String) -> Void  // user-visible status/warning messages
    // written by cancel() from the UI/cleanup path, read by the discovery tasks
    let cancelled = AtomicBool(false)

    init(key: [UInt8], role: DiscoveryRole, localIP: String, output: @escaping (String) -> Void) {
        self.key = key
        self.role = role
        self.localIP = localIP
        self.output = output
    }

    func cancel() {
        cancelled.value = true
    }

    /// Sender role: returns the receiver's IP once a valid announcement arrives.
    /// Receiver role: never returns a peer — it announces our presence and surfaces
    /// diagnostics until cancelled, because the receiver's real completion signal is
    /// the sender's TCP connection (the sender stops announcing as soon as it hears
    /// us, possibly before we ever hear it, so waiting to hear the sender can deadlock).
    func discoverPeer() async throws -> String {
        print("[Discovery] Starting peer discovery with unified receiver")

        // Compute subnet base for unicast scanning
        let octets = localIP.split(separator: ".").compactMap { UInt8($0) }
        guard octets.count == 4 else {
            throw DiscoveryError.noLocalIP
        }
        let baseIP = "\(octets[0]).\(octets[1]).\(octets[2])"

        return try await withThrowingTaskGroup(of: String?.self) { group in
            // Unified receiver: single POSIX UDP socket that receives both
            // multicast and unicast announcements on DISCOVERY_PORT.
            // This avoids the port conflict that occurs when NWConnectionGroup
            // and NWListener both try to bind to the same port.
            group.addTask {
                return try await self.receiveAnnouncements()
            }

            // Multicast sender (never throws, so a multicast failure can't kill discovery)
            group.addTask {
                await self.sendMulticastAnnouncements()
                return nil
            }

            // Unicast sender (subnet scan; never throws)
            group.addTask {
                await self.sendUnicastProbes(baseIP: baseIP)
                return nil
            }

            // No timeout: the user may start this device long before the other, so keep
            // searching until the peer appears or the transfer is cancelled.
            for try await result in group {
                if let peerIP = result {
                    print("[Discovery] Found peer: \(peerIP)")
                    group.cancelAll()
                    return peerIP
                }
            }

            throw DiscoveryError.cancelled
        }
    }

    // MARK: - Unified Receiver (Multicast + Unicast)

    /// Receives both multicast and unicast discovery announcements on a single UDP socket.
    /// Uses POSIX sockets directly to mirror the Rust implementation, which binds one socket
    /// to 0.0.0.0:DISCOVERY_PORT and joins the multicast group on it. This avoids the port
    /// conflict caused by NWConnectionGroup and NWListener both binding to the same port.
    private func receiveAnnouncements() async throws -> String {
        let fd = socket(AF_INET, SOCK_DGRAM, 0)
        guard fd >= 0 else {
            throw DiscoveryError.networkError("Failed to create UDP socket: errno \(errno)")
        }
        defer { Darwin.close(fd) }

        // Allow port reuse (matches Rust's set_reuse_address)
        var yes: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout<Int32>.size))
        setsockopt(fd, SOL_SOCKET, SO_REUSEPORT, &yes, socklen_t(MemoryLayout<Int32>.size))

        // Bind to 0.0.0.0:DISCOVERY_PORT
        var bindAddr = sockaddr_in()
        bindAddr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        bindAddr.sin_family = sa_family_t(AF_INET)
        bindAddr.sin_port = DISCOVERY_PORT.bigEndian
        bindAddr.sin_addr.s_addr = INADDR_ANY

        let bindResult = withUnsafePointer(to: &bindAddr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPtr in
                Darwin.bind(fd, sockaddrPtr, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard bindResult == 0 else {
            throw DiscoveryError.networkError("Failed to bind to port \(DISCOVERY_PORT): \(String(cString: strerror(errno)))")
        }

        // Join multicast group on the WiFi interface
        var mreq = ip_mreq()
        mreq.imr_multiaddr.s_addr = inet_addr(MULTICAST_ADDR)
        mreq.imr_interface.s_addr = inet_addr(localIP)
        let joinResult = setsockopt(fd, IPPROTO_IP, IP_ADD_MEMBERSHIP, &mreq, socklen_t(MemoryLayout<ip_mreq>.size))
        if joinResult != 0 {
            print("[Discovery] Warning: Failed to join multicast group: \(String(cString: strerror(errno)))")
            // Continue — unicast discovery will still work
        }

        // Disable multicast loopback (don't receive our own multicast packets)
        var loopOff: UInt8 = 0
        setsockopt(fd, IPPROTO_IP, IP_MULTICAST_LOOP, &loopOff, socklen_t(MemoryLayout<UInt8>.size))

        // Non-blocking mode for async polling
        let flags = fcntl(fd, F_GETFL)
        fcntl(fd, F_SETFL, flags | O_NONBLOCK)

        // Receive loop. There's no discovery timeout (the other device may not be started
        // for a long time), so problems are reported inline, once each, while the search
        // continues.
        var buf = [UInt8](repeating: 0, count: 1024)
        let localIP = self.localIP
        let key = self.key
        let role = self.role
        let output = self.output
        let started = Date()
        var receivedPeerPacket = false
        var foundSender = false
        var warnedQuiet = false
        var warnedHmac = false
        var warnedStale = false
        var warnedRole = false

        while !cancelled.value && !Task.isCancelled {
            var srcAddr = sockaddr_in()
            var addrLen = socklen_t(MemoryLayout<sockaddr_in>.size)

            let bytesRead = withUnsafeMutablePointer(to: &srcAddr) { ptr in
                ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPtr in
                    recvfrom(fd, &buf, buf.count, 0, sockaddrPtr, &addrLen)
                }
            }

            if bytesRead > 0 {
                let data = Data(bytes: buf, count: bytesRead)
                print("[Discovery] Received \(bytesRead) bytes")

                guard let announcement = DiscoveryAnnouncement.deserialize(data) else {
                    print("[Discovery] Failed to deserialize announcement")
                    continue
                }
                print("[Discovery] From \(announcement.getIPString()), role: \(announcement.role)")

                if announcement.getIPString() == localIP { continue }  // our own announcement
                receivedPeerPacket = true
                guard announcement.verify(key: key) else {
                    if !warnedHmac {
                        warnedHmac = true
                        output("Received an announcement that failed authentication. If it came from the other device, check that the password matches on both. Still searching...")
                    }
                    print("[Discovery] HMAC verification failed")
                    continue
                }
                guard announcement.isTimestampValid() else {
                    if !warnedStale {
                        warnedStale = true
                        output("Received an announcement with an out-of-date timestamp. Check that both devices' clocks are set correctly. Still searching...")
                    }
                    print("[Discovery] Timestamp invalid")
                    continue
                }
                guard announcement.role == role.opposite else {
                    if !warnedRole {
                        warnedRole = true
                        output("Received an announcement from a device in the same mode as this one. If it's the other device of this transfer, one side must select Send and the other Receive. Still searching...")
                    }
                    print("[Discovery] Role mismatch")
                    continue
                }

                print("[Discovery] Found valid peer at \(announcement.getIPString())")
                if role == .receiver {
                    // The receiver's completion signal is the sender's TCP connection,
                    // not discovery. Keep announcing so the sender can find us; this
                    // is informational only.
                    if !foundSender {
                        foundSender = true
                        output("Found the sender at \(announcement.getIPString()). Waiting for it to connect...")
                    }
                    continue
                }
                return announcement.getIPString()
            } else if bytesRead < 0 && errno != EAGAIN && errno != EWOULDBLOCK {
                throw DiscoveryError.networkError("recvfrom failed: \(String(cString: strerror(errno)))")
            }

            // If nothing from a peer has arrived after a while, hint at likely causes
            // (the other device may also simply not be started yet).
            if !warnedQuiet && !receivedPeerPacket && Date().timeIntervalSince(started) >= 30 {
                warnedQuiet = true
                output("Still searching. If the other device has already started the transfer, check that both devices are on the same network and that no firewall is blocking UDP port 3290.")
            }

            // Brief sleep to avoid busy-waiting
            try await Task.sleep(nanoseconds: 50_000_000) // 50ms poll interval
        }

        throw DiscoveryError.cancelled
    }

    // MARK: - Send helpers (POSIX UDP)

    /// Builds a sockaddr_in for the given dotted-quad IP and port.
    private func makeSockaddr(ip: String, port: UInt16) -> sockaddr_in {
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        addr.sin_addr.s_addr = inet_addr(ip)
        return addr
    }

    /// Sends `data` to `addr` on the given socket.
    private func sendTo(fd: Int32, data: Data, addr: sockaddr_in) {
        var addr = addr
        _ = data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            withUnsafePointer(to: &addr) { ptr in
                ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPtr in
                    sendto(fd, raw.baseAddress, data.count, 0, sockaddrPtr, socklen_t(MemoryLayout<sockaddr_in>.size))
                }
            }
        }
    }

    // MARK: - Multicast Sender

    // Uses a raw UDP socket (mirrors the Rust implementation) instead of an
    // NWConnection, so a multicast setup failure on a network that blocks multicast
    // can't throw out of the discovery task group and kill the unicast fallback.
    private func sendMulticastAnnouncements() async {
        let fd = socket(AF_INET, SOCK_DGRAM, 0)
        guard fd >= 0 else {
            print("[Discovery] Failed to create multicast send socket: errno \(errno)")
            return
        }
        defer { Darwin.close(fd) }

        // Egress multicast on the WiFi interface.
        var ifAddr = in_addr()
        ifAddr.s_addr = inet_addr(localIP)
        setsockopt(fd, IPPROTO_IP, IP_MULTICAST_IF, &ifAddr, socklen_t(MemoryLayout<in_addr>.size))

        let dest = makeSockaddr(ip: MULTICAST_ADDR, port: DISCOVERY_PORT)
        var sequence: UInt32 = 0
        print("[Discovery] Starting to send multicast announcements as \(role)")

        while !cancelled.value && !Task.isCancelled {
            var announcement = DiscoveryAnnouncement(role: role, ipAddress: localIP, sequence: sequence)
            announcement.sign(key: key)
            sendTo(fd: fd, data: announcement.serialize(), addr: dest)

            if sequence % 10 == 0 {
                print("[Discovery] Sent announcement #\(sequence) from \(localIP) as \(role)")
            }
            sequence = sequence &+ 1
            try? await Task.sleep(nanoseconds: DISCOVERY_INTERVAL_MS * 1_000_000)
        }
    }

    // MARK: - Unicast Sender

    // Single UDP socket reused for the whole subnet scan (one sendto per host),
    // instead of creating an NWConnection per IP per round.
    private func sendUnicastProbes(baseIP: String) async {
        let fd = socket(AF_INET, SOCK_DGRAM, 0)
        guard fd >= 0 else {
            print("[Discovery/Unicast] Failed to create send socket: errno \(errno)")
            return
        }
        defer { Darwin.close(fd) }

        var sequence: UInt32 = 0
        print("[Discovery/Unicast] Starting subnet scan on \(baseIP).x")

        while !cancelled.value && !Task.isCancelled {
            // Sign one announcement per round and reuse its bytes for every host.
            var announcement = DiscoveryAnnouncement(role: role, ipAddress: localIP, sequence: sequence)
            announcement.sign(key: key)
            let data = announcement.serialize()

            for i in 1...254 {
                if cancelled.value || Task.isCancelled { return }
                let targetIP = "\(baseIP).\(i)"
                if targetIP == localIP { continue }
                sendTo(fd: fd, data: data, addr: makeSockaddr(ip: targetIP, port: DISCOVERY_PORT))
            }

            sequence = sequence &+ 1
            try? await Task.sleep(nanoseconds: DISCOVERY_INTERVAL_MS * 1_000_000)
        }
    }

}
