//
//  Noise.swift
//  FlyingCarpet
//
//  v10+ encrypted transport for all transfers (both hotspot and shared network mode). A
//  hand-rolled implementation of the Noise NNpsk0 handshake and ChaChaPoly transport,
//  byte-for-byte compatible with the Rust reference (core/src/noise.rs) and the Android
//  reference (Noise.kt). See docs/shared-network-crypto.md in the main FlyingCarpet repo.
//
//  Hand-rolled (rather than a Noise library) because CryptoKit has no Noise/PAKE support and
//  the "Apple standard crypto only" constraint rules out third-party crypto. All primitives
//  come from CryptoKit (X25519, ChaCha20-Poly1305, SHA-256, HMAC) and CommonCrypto (PBKDF2).
//  Verified byte-for-byte against the official cacophony test vector in NoiseTests.
//

import Foundation
import CryptoKit
import CommonCrypto
import Network

// Must be byte-identical across Rust, Swift, and Kotlin.
let NOISE_PROTOCOL_NAME = "Noise_NNpsk0_25519_ChaChaPoly_SHA256"
let PSK_SALT = Data("Flying Carpet v10 shared network PSK".utf8)
let PBKDF2_ITERS = 600_000
// Domain-separation label for the discovery announcement HMAC key (derived from the PSK,
// never from a fast hash of the password — see deriveDiscoveryKey).
let DISCOVERY_INFO = Data("Flying Carpet v10 discovery".utf8)
let NOISE_TAG_LEN = 16
let NOISE_MAX_MESSAGE = 65535
let NOISE_MAX_PLAINTEXT = NOISE_MAX_MESSAGE - NOISE_TAG_LEN // 65519

enum NoiseRole {
    case initiator // shared network sender / hotspot guest (TCP client)
    case responder // shared network receiver / hotspot host (TCP server)
}

struct NoiseError: Error, CustomStringConvertible {
    let description: String
}

// MARK: - Primitives

private func sha256(_ data: Data) -> Data {
    return Data(SHA256.hash(data: data))
}

private func hmacSha256(key: Data, data: Data) -> Data {
    let mac = HMAC<SHA256>.authenticationCode(for: data, using: SymmetricKey(data: key))
    return Data(mac)
}

// Noise HKDF: `num` (2 or 3) 32-byte outputs from the chaining key and IKM.
private func noiseHKDF(ck: Data, ikm: Data, num: Int) -> [Data] {
    let tempKey = hmacSha256(key: ck, data: ikm)
    let o1 = hmacSha256(key: tempKey, data: Data([0x01]))
    let o2 = hmacSha256(key: tempKey, data: o1 + Data([0x02]))
    if num == 2 { return [o1, o2] }
    let o3 = hmacSha256(key: tempKey, data: o2 + Data([0x03]))
    return [o1, o2, o3]
}

// X25519 shared secret (raw, unhashed). CryptoKit's SharedSecret for Curve25519 is the raw
// scalar-multiplication output, which is exactly Noise's DH.
private func x25519(privateKey: Data, publicKey: Data) throws -> Data {
    let priv = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: privateKey)
    let pub = try Curve25519.KeyAgreement.PublicKey(rawRepresentation: publicKey)
    let shared = try priv.sharedSecretFromKeyAgreement(with: pub)
    return shared.withUnsafeBytes { Data($0) }
}

private func publicKey(fromPrivate privateKey: Data) throws -> Data {
    return try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: privateKey).publicKey.rawRepresentation
}

/// Builds the canonical Noise prologue from the plaintext preamble transcript.
/// `initiatorTranscript` is every byte the Noise initiator sent during the preamble
/// (version + mode exchange) and `responderTranscript` every byte the responder sent; each
/// is length-prefixed (u64 big-endian, matching the app's length idiom) so the encoding is
/// unambiguous. Each side computes this from its own sent/received bytes — the initiator as
/// (sent, received), the responder as (received, sent) — so any in-flight tampering with
/// the preamble makes the prologues differ, which fails the handshake. Must be
/// byte-identical across Rust, Swift, and Kotlin (see core/src/noise.rs build_prologue).
func buildPrologue(initiatorTranscript: Data, responderTranscript: Data) -> Data {
    func be64(_ n: Int) -> Data {
        var d = Data(count: 8)
        for i in 0..<8 { d[i] = UInt8((UInt64(n) >> (8 * UInt64(7 - i))) & 0xff) }
        return d
    }
    return be64(initiatorTranscript.count) + initiatorTranscript
        + be64(responderTranscript.count) + responderTranscript
}

/// Wraps the TCP connection during the plaintext preamble, recording every byte sent and
/// received so the transcript can be bound into the Noise prologue (see buildPrologue).
/// Recording at the connection boundary — rather than inside the preamble functions —
/// means no exchanged byte can be missed, whatever branch the version/mode negotiation
/// takes.
final class RecordingTCPConnection: TCPConnectionProtocol {
    let inner: any TCPConnectionProtocol
    private(set) var sent = Data()
    private(set) var received = Data()

    init(inner: any TCPConnectionProtocol) {
        self.inner = inner
    }

    var connection: NWConnection { inner.connection }

    func write(data: Data) async throws {
        try await inner.write(data: data)
        sent.append(data)
    }

    func receiveNBytes(n: Int) async throws -> Data {
        let data = try await inner.receiveNBytes(n: n)
        received.append(data)
        return data
    }

    func disconnect() { inner.disconnect() }
    func forceDisconnect() { inner.forceDisconnect() }
}

/// Derives the 32-byte Noise pre-shared key from the transfer password with
/// PBKDF2-HMAC-SHA256 over the UTF-8 password bytes (CommonCrypto). Byte-identical to the
/// Rust/Kotlin `derivePsk`.
func derivePsk(_ password: String) -> Data {
    let pw = Data(password.utf8)
    var derived = Data(repeating: 0, count: 32)
    let status = derived.withUnsafeMutableBytes { derivedPtr -> Int32 in
        PSK_SALT.withUnsafeBytes { saltPtr -> Int32 in
            pw.withUnsafeBytes { pwPtr -> Int32 in
                CCKeyDerivationPBKDF(
                    CCPBKDFAlgorithm(kCCPBKDF2),
                    pwPtr.bindMemory(to: Int8.self).baseAddress, pw.count,
                    saltPtr.bindMemory(to: UInt8.self).baseAddress, PSK_SALT.count,
                    CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA256),
                    UInt32(PBKDF2_ITERS),
                    derivedPtr.bindMemory(to: UInt8.self).baseAddress, 32
                )
            }
        }
    }
    precondition(status == kCCSuccess, "PBKDF2 failed: \(status)")
    return derived
}

/// Derives the discovery announcement HMAC key from the PBKDF2-stretched PSK, with a
/// fixed label for domain separation (the Noise PSK itself is never used outside the
/// handshake). Keying discovery from the stretched PSK — instead of a fast
/// SHA256(password) — means a captured announcement costs an offline attacker 600k
/// PBKDF2 iterations per password guess, the same as the handshake, so the password
/// can't be cracked while it's still live. Must be byte-identical across Rust, Swift,
/// and Kotlin (see core/src/noise.rs derive_discovery_key).
func deriveDiscoveryKey(psk: Data) -> Data {
    return hmacSha256(key: psk, data: DISCOVERY_INFO)
}

// MARK: - Transport cipher (ChaCha20-Poly1305 with the Noise nonce)

final class NoiseCipherState {
    private let key: SymmetricKey
    private var nonce: UInt64 = 0

    init(key: Data) {
        self.key = SymmetricKey(data: key)
    }

    // Noise 96-bit nonce: 32 bits of zero followed by the 64-bit little-endian counter.
    private func makeNonce() -> ChaChaPoly.Nonce {
        var bytes = [UInt8](repeating: 0, count: 12)
        for i in 0..<8 { bytes[4 + i] = UInt8((nonce >> (8 * UInt64(i))) & 0xff) }
        return try! ChaChaPoly.Nonce(data: Data(bytes))
    }

    func encrypt(ad: Data, plaintext: Data) throws -> Data {
        let box = try ChaChaPoly.seal(plaintext, using: key, nonce: makeNonce(), authenticating: ad)
        nonce += 1
        return box.ciphertext + box.tag
    }

    func decrypt(ad: Data, ciphertext: Data) throws -> Data {
        guard ciphertext.count >= NOISE_TAG_LEN else {
            throw NoiseError(description: "Noise record too short")
        }
        let c = Data(ciphertext)
        let ct = c.subdata(in: 0..<(c.count - NOISE_TAG_LEN))
        let tag = c.subdata(in: (c.count - NOISE_TAG_LEN)..<c.count)
        let box = try ChaChaPoly.SealedBox(nonce: makeNonce(), ciphertext: ct, tag: tag)
        let plain = try ChaChaPoly.open(box, using: key, authenticating: ad)
        nonce += 1
        return plain
    }
}

// MARK: - Noise symmetric state

private final class SymmetricState {
    var ck: Data
    var h: Data
    private var cs: NoiseCipherState?

    init() {
        let name = Data(NOISE_PROTOCOL_NAME.utf8)
        if name.count <= 32 {
            h = name + Data(repeating: 0, count: 32 - name.count)
        } else {
            h = sha256(name)
        }
        ck = h
    }

    func mixHash(_ data: Data) {
        h = sha256(h + data)
    }

    func mixKey(_ input: Data) {
        let out = noiseHKDF(ck: ck, ikm: input, num: 2)
        ck = out[0]
        cs = NoiseCipherState(key: out[1])
    }

    func mixKeyAndHash(_ input: Data) {
        let out = noiseHKDF(ck: ck, ikm: input, num: 3)
        ck = out[0]
        mixHash(out[1])
        cs = NoiseCipherState(key: out[2])
    }

    func encryptAndHash(_ plaintext: Data) throws -> Data {
        let ct = try cs?.encrypt(ad: h, plaintext: plaintext) ?? plaintext
        mixHash(ct)
        return ct
    }

    func decryptAndHash(_ ciphertext: Data) throws -> Data {
        let pt = try cs?.decrypt(ad: h, ciphertext: ciphertext) ?? ciphertext
        mixHash(ciphertext)
        return pt
    }

    func split() -> (NoiseCipherState, NoiseCipherState) {
        let out = noiseHKDF(ck: ck, ikm: Data(), num: 2)
        return (NoiseCipherState(key: out[0]), NoiseCipherState(key: out[1]))
    }
}

// MARK: - NNpsk0 handshake
//
// Messages: `-> psk, e` then `<- e, ee`. In PSK mode the `e` token additionally MixKey's the
// ephemeral public key. Testable independently of sockets: NoiseTests drives this with fixed
// ephemerals to check message bytes against the official cacophony vector.
final class NoiseHandshakeState {
    private let role: NoiseRole
    private let psk: Data
    private let ss = SymmetricState()
    private let ePriv: Data
    private let ePub: Data
    private var rePub: Data?
    private var messageIndex = 0

    init(role: NoiseRole, psk: Data, prologue: Data = Data(), fixedEphemeral: Data? = nil) throws {
        self.role = role
        self.psk = psk
        self.ePriv = fixedEphemeral ?? Curve25519.KeyAgreement.PrivateKey().rawRepresentation
        self.ePub = try publicKey(fromPrivate: ePriv)
        ss.mixHash(prologue)
    }

    func writeMessage(_ payload: Data = Data()) throws -> Data {
        var out = Data()
        if messageIndex == 0 {
            // -> psk, e
            ss.mixKeyAndHash(psk)
            out.append(ePub)
            ss.mixHash(ePub)
            ss.mixKey(ePub)
        } else {
            // <- e, ee
            out.append(ePub)
            ss.mixHash(ePub)
            ss.mixKey(ePub)
            ss.mixKey(try x25519(privateKey: ePriv, publicKey: rePub!))
        }
        out.append(try ss.encryptAndHash(payload))
        messageIndex += 1
        return out
    }

    func readMessage(_ message: Data) throws -> Data {
        let m = Data(message)
        guard m.count >= 32 else { throw NoiseError(description: "Noise handshake message too short") }
        let re = m.subdata(in: 0..<32)
        if messageIndex == 0 {
            // -> psk, e
            ss.mixKeyAndHash(psk)
            rePub = re
            ss.mixHash(re)
            ss.mixKey(re)
        } else {
            // <- e, ee
            rePub = re
            ss.mixHash(re)
            ss.mixKey(re)
            ss.mixKey(try x25519(privateKey: ePriv, publicKey: re))
        }
        let payload = try ss.decryptAndHash(m.subdata(in: 32..<m.count))
        messageIndex += 1
        return payload
    }

    var handshakeHash: Data { ss.h }

    // Returns (send cipher, receive cipher) oriented for this party's role.
    func split() -> (send: NoiseCipherState, recv: NoiseCipherState) {
        let (c1, c2) = ss.split()
        return role == .initiator ? (c1, c2) : (c2, c1)
    }
}

// MARK: - Framing + handshake over a TCP connection

// Each Noise message (handshake or transport record) is prefixed with its length as a
// 2-byte big-endian integer; 2 bytes because Noise caps a message at 65535 bytes.
private func writeNoiseFrame(_ tcp: any TCPConnectionProtocol, _ msg: Data) async throws {
    var frame = Data()
    frame.append(UInt8((msg.count >> 8) & 0xff))
    frame.append(UInt8(msg.count & 0xff))
    frame.append(msg)
    try await tcp.write(data: frame)
}

private func readNoiseFrame(_ tcp: any TCPConnectionProtocol) async throws -> Data {
    let lenBytes = try await tcp.receiveNBytes(n: 2)
    let b = Data(lenBytes)
    let len = (Int(b[0]) << 8) | Int(b[1])
    return try await tcp.receiveNBytes(n: len)
}

/// Runs the Noise NNpsk0 handshake over `tcp` and returns a NoiseConnection wrapping it.
/// The initiator (TCP client) sends the first message. `psk` is the PBKDF2-stretched
/// password from derivePsk, derived once by the caller (in shared network mode it also
/// keys discovery). `prologue` binds the plaintext preamble transcript (see
/// buildPrologue): if either the password or the preamble bytes differ between the
/// peers, the first authenticated message fails to decrypt.
func noiseHandshake(tcp: any TCPConnectionProtocol, role: NoiseRole, psk: Data, prologue: Data) async throws -> NoiseConnection {
    let hs = try NoiseHandshakeState(role: role, psk: psk, prologue: prologue)
    do {
        if role == .initiator {
            try await writeNoiseFrame(tcp, try hs.writeMessage())
            _ = try hs.readMessage(try await readNoiseFrame(tcp))
        } else {
            _ = try hs.readMessage(try await readNoiseFrame(tcp))
            try await writeNoiseFrame(tcp, try hs.writeMessage())
        }
    } catch CryptoKitError.authenticationFailure {
        throw NoiseError(description: "Could not establish a secure connection. Check that the password matches on both devices. (This can also mean the connection was tampered with.)")
    }
    let (send, recv) = hs.split()
    return NoiseConnection(inner: tcp, sendCipher: send, recvCipher: recv)
}

// MARK: - Encrypted connection (a TCPConnectionProtocol the transfer code uses unchanged)

final class NoiseConnection: TCPConnectionProtocol {
    private let inner: any TCPConnectionProtocol
    private let sendCipher: NoiseCipherState
    private let recvCipher: NoiseCipherState
    private var plainBuffer = Data()

    init(inner: any TCPConnectionProtocol, sendCipher: NoiseCipherState, recvCipher: NoiseCipherState) {
        self.inner = inner
        self.sendCipher = sendCipher
        self.recvCipher = recvCipher
    }

    // TCPConnectionProtocol requirement; the protocol extension's default write/receiveNBytes
    // use it, but on this class both are overridden below, so it's only exposed for teardown.
    var connection: NWConnection { inner.connection }

    func write(data: Data) async throws {
        let d = Data(data)
        var offset = 0
        while offset < d.count {
            let take = min(d.count - offset, NOISE_MAX_PLAINTEXT)
            let chunk = d.subdata(in: offset..<(offset + take))
            let record = try sendCipher.encrypt(ad: Data(), plaintext: chunk)
            try await writeNoiseFrame(inner, record)
            offset += take
        }
    }

    func receiveNBytes(n: Int) async throws -> Data {
        while plainBuffer.count < n {
            let record = try await readNoiseFrame(inner)
            let plain = try recvCipher.decrypt(ad: Data(), ciphertext: record)
            plainBuffer.append(plain)
        }
        let result = Data(plainBuffer.prefix(n))
        plainBuffer.removeFirst(n)
        return result
    }

    func disconnect() { inner.disconnect() }
    func forceDisconnect() { inner.forceDisconnect() }
}
