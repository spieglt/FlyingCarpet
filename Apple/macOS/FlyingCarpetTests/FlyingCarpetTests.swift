//
//  FlyingCarpetTests.swift
//  FlyingCarpetTests
//
//  Created by Theron on 6/9/24.
//

import XCTest
import CryptoKit
@testable import FlyingCarpet

final class FlyingCarpetTests: XCTestCase {

    override func setUpWithError() throws {
        // Put setup code here. This method is called before the invocation of each test method in the class.
    }

    override func tearDownWithError() throws {
        // Put teardown code here. This method is called after the invocation of each test method in the class.
    }

    func testExample() throws {
        // This is an example of a functional test case.
        // Use XCTAssert and related functions to verify your tests produce the correct results.
        // Any test you write for XCTest can be annotated as throws and async.
        // Mark your test throws to produce an unexpected failure when your test encounters an uncaught error.
        // Mark your test async to allow awaiting for asynchronous code to complete. Check the results with assertions afterwards.
    }

    func testPerformanceExample() throws {
        // This is an example of a performance test case.
        self.measure {
            // Put the code you want to measure the time of here.
        }
    }

}

// Cross-platform verification of the hand-rolled Noise implementation (Noise.swift). These
// KATs are shared with the Rust reference (core/src/noise.rs) and the Android reference
// (Noise.kt): passing them proves this Swift port interoperates with the other two.
final class NoiseTests: XCTestCase {

    private func hex(_ s: String) -> Data {
        var d = Data()
        var i = s.startIndex
        while i < s.endIndex {
            let j = s.index(i, offsetBy: 2)
            d.append(UInt8(s[i..<j], radix: 16)!)
            i = j
        }
        return d
    }

    private func toHex(_ d: Data) -> String {
        d.map { String(format: "%02x", $0) }.joined()
    }

    // Spec conformance: drive the handshake with the official cacophony test vector for
    // Noise_NNpsk0_25519_ChaChaPoly_SHA256 and assert the handshake message ciphertexts, the
    // handshake hash, and the first transport record match byte-for-byte.
    func testOfficialNoiseVector() throws {
        let prologue = hex("4a6f686e2047616c74") // "John Galt"
        let psk = hex("54686973206973206d7920417573747269616e20706572737065637469766521")
        let initEph = hex("893e28b9dc6ca8d611ab664754b8ceb7bac5117349a4439a6b0569da977c464a")
        let respEph = hex("bbdb4cdbd309f1a1f2e1456967fe288cadd6f712d65dc7b7793d5e63da6b375b")

        let initHS = try NoiseHandshakeState(role: .initiator, psk: psk, prologue: prologue, fixedEphemeral: initEph)
        let respHS = try NoiseHandshakeState(role: .responder, psk: psk, prologue: prologue, fixedEphemeral: respEph)

        let p0 = hex("4c756477696720766f6e204d69736573") // "Ludwig von Mises"
        let msg0 = try initHS.writeMessage(p0)
        XCTAssertEqual(toHex(msg0), "ca35def5ae56cec33dc2036731ab14896bc4c75dbb07a61f879f8e3afa4c794479b962b8aff8485742ac32f905ba45369e2465fb59e138a93d67a0d1266b6a54")
        XCTAssertEqual(try respHS.readMessage(msg0), p0)

        let p1 = hex("4d757272617920526f746862617264") // "Murray Rothbard"
        let msg1 = try respHS.writeMessage(p1)
        XCTAssertEqual(toHex(msg1), "95ebc60d2b1fa672c1f46a8aa265ef51bfe38e7ccb39ec5be34069f144808843d6062704d5a9c422a8e834423f8c1feada7e8d0d910a1a2cd030fb584221e3")
        XCTAssertEqual(try initHS.readMessage(msg1), p1)

        let hh = "f4d03dc34495c95729ea6de9e1b59004b59733102488b3e24bc441e0be208eaf"
        XCTAssertEqual(toHex(initHS.handshakeHash), hh)
        XCTAssertEqual(toHex(respHS.handshakeHash), hh)

        let (initSend, _) = initHS.split()
        let (_, respRecv) = respHS.split()
        let p2 = hex("462e20412e20486179656b") // "F. A. Hayek"
        let record = try initSend.encrypt(ad: Data(), plaintext: p2)
        XCTAssertEqual(toHex(record), "e632c3763d7669067383433197a3baddf146e9e70ad4b4e9e59e0f")
        XCTAssertEqual(try respRecv.decrypt(ad: Data(), ciphertext: record), p2)
    }

    // Cross-platform known-answer for the PSK derivation (must match Rust's derive_psk).
    func testPskKnownAnswer() {
        XCTAssertEqual(toHex(derivePsk("flyingcarpet")),
                       "a3d8b7f17f2252e4c2847a365ab2f392beaa996b7e51dd6fa19ff1ad08938619")
    }

    // Cross-platform known-answer for the discovery announcement HMAC key: HMAC-SHA256
    // keyed by the PSK over the fixed DISCOVERY_INFO label (must match Rust's
    // derive_discovery_key and Android's deriveDiscoveryKey, docs §9).
    func testDiscoveryKeyKnownAnswer() {
        XCTAssertEqual(toHex(deriveDiscoveryKey(psk: derivePsk("flyingcarpet"))),
                       "45e49b632788b21069bf48720d6af230ecbd936b3cb16c898a8e1eac51944112")
    }

    // App-parameter known-answer (empty prologue, fixed PSK/ephemerals) matching the Rust
    // reference's handshake_known_answer (docs §9).
    func testAppHandshakeKnownAnswer() throws {
        let psk = Data(repeating: 0x2a, count: 32)
        let initEph = Data(repeating: 0x01, count: 32)
        let respEph = Data(repeating: 0x02, count: 32)

        let initHS = try NoiseHandshakeState(role: .initiator, psk: psk, prologue: Data(), fixedEphemeral: initEph)
        let respHS = try NoiseHandshakeState(role: .responder, psk: psk, prologue: Data(), fixedEphemeral: respEph)

        let msg1 = try initHS.writeMessage()
        XCTAssertEqual(toHex(msg1), "a4e09292b651c278b9772c569f5fa9bb13d906b46ab68c9df9dc2b4409f8a209a3e9c18456aba2185de800ffaca55b22")
        _ = try respHS.readMessage(msg1)

        let msg2 = try respHS.writeMessage()
        XCTAssertEqual(toHex(msg2), "ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59d887595caf8a0b110dfab84e6b41eafc")
        _ = try initHS.readMessage(msg2)

        let (initSend, _) = initHS.split()
        let record = try initSend.encrypt(ad: Data(), plaintext: Data("hello flying carpet".utf8))
        XCTAssertEqual(toHex(record), "124a00c03b4544f746828bbf9ae2d8d595a9ac1fea988f43f7206c3880180b954f9147")
    }

    private func handshakeInMemory(_ password: String) throws -> (NoiseCipherState, NoiseCipherState) {
        let psk = derivePsk(password)
        let initHS = try NoiseHandshakeState(role: .initiator, psk: psk)
        let respHS = try NoiseHandshakeState(role: .responder, psk: psk)
        _ = try respHS.readMessage(initHS.writeMessage())
        _ = try initHS.readMessage(respHS.writeMessage())
        let (initSend, _) = initHS.split()
        let (_, respRecv) = respHS.split()
        return (initSend, respRecv)
    }

    // Encrypt a payload larger than one Noise record (splitting like NoiseConnection does) and
    // decrypt it back.
    func testTransportRoundTripMultiRecord() throws {
        let (send, recv) = try handshakeInMemory("correct horse")
        let big = Data((0..<200_000).map { UInt8($0 % 251) })

        var records = [Data]()
        var offset = 0
        while offset < big.count {
            let take = min(big.count - offset, NOISE_MAX_PLAINTEXT)
            records.append(try send.encrypt(ad: Data(), plaintext: big.subdata(in: offset..<(offset + take))))
            offset += take
        }
        var got = Data()
        for record in records {
            got.append(try recv.decrypt(ad: Data(), ciphertext: record))
        }
        XCTAssertEqual(got, big)
    }

    // A mismatched password makes the responder's read of the first message fail.
    func testWrongPasswordFails() throws {
        let initHS = try NoiseHandshakeState(role: .initiator, psk: derivePsk("password-one"))
        let respHS = try NoiseHandshakeState(role: .responder, psk: derivePsk("password-two"))
        let msg0 = try initHS.writeMessage()
        XCTAssertThrowsError(try respHS.readMessage(msg0))
    }

    // Corrupting any single byte of a transport record must make the receiver's decrypt
    // fail (the Poly1305 tag no longer verifies).
    func testTamperingIsDetected() throws {
        let (send, recv) = try handshakeInMemory("shared secret")

        // first record, untampered: decrypts fine (sanity-checks the setup)
        let ok = try send.encrypt(ad: Data(), plaintext: Data("authentic payload".utf8))
        XCTAssertEqual(try recv.decrypt(ad: Data(), ciphertext: ok), Data("authentic payload".utf8))

        // second record with one bit flipped in the ciphertext: must fail to authenticate
        var record = try send.encrypt(ad: Data(), plaintext: Data("authentic payload".utf8))
        record[record.count / 2] ^= 0x01
        XCTAssertThrowsError(try recv.decrypt(ad: Data(), ciphertext: record))
    }

    // Same guarantee for a corrupted handshake message: the responder must reject it.
    func testTamperedHandshakeIsDetected() throws {
        let psk = derivePsk("shared secret")
        let initHS = try NoiseHandshakeState(role: .initiator, psk: psk)
        let respHS = try NoiseHandshakeState(role: .responder, psk: psk)
        var msg0 = try initHS.writeMessage()
        // corrupt the encrypted payload section (past the 32-byte ephemeral)
        msg0[msg0.count - 1] ^= 0x01
        XCTAssertThrowsError(try respHS.readMessage(msg0))
    }

    // Cross-platform known-answer for the prologue-bound handshake, matching the Rust
    // reference's prologue_known_answer (docs §9): the app-style preamble transcript
    // (version 10 + mode, 8-byte big-endian each) framed by buildPrologue, with fixed
    // PSK/ephemerals. The transport record intentionally matches testAppHandshakeKnownAnswer's:
    // per the Noise spec the prologue only enters h (gating the handshake MACs), never the
    // chaining key — asserted anyway so a port that wrongly mixes it into ck fails here.
    func testPrologueKnownAnswer() throws {
        let initTranscript = hex("000000000000000a0000000000000001")
        let respTranscript = hex("000000000000000a0000000000000000")
        let prologue = buildPrologue(initiatorTranscript: initTranscript, responderTranscript: respTranscript)
        XCTAssertEqual(toHex(prologue), "0000000000000010000000000000000a00000000000000010000000000000010000000000000000a0000000000000000")

        let psk = Data(repeating: 0x2a, count: 32)
        let initHS = try NoiseHandshakeState(role: .initiator, psk: psk, prologue: prologue, fixedEphemeral: Data(repeating: 0x01, count: 32))
        let respHS = try NoiseHandshakeState(role: .responder, psk: psk, prologue: prologue, fixedEphemeral: Data(repeating: 0x02, count: 32))

        let msg1 = try initHS.writeMessage()
        XCTAssertEqual(toHex(msg1), "a4e09292b651c278b9772c569f5fa9bb13d906b46ab68c9df9dc2b4409f8a2093ae03dc8524f79ac9696d6c155df9a3c")
        _ = try respHS.readMessage(msg1)

        let msg2 = try respHS.writeMessage()
        XCTAssertEqual(toHex(msg2), "ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59d2668070263116ce557500fbe3fd3ba4")
        _ = try initHS.readMessage(msg2)

        let (initSend, _) = initHS.split()
        let record = try initSend.encrypt(ad: Data(), plaintext: Data("hello flying carpet".utf8))
        XCTAssertEqual(toHex(record), "124a00c03b4544f746828bbf9ae2d8d595a9ac1fea988f43f7206c3880180b954f9147")
    }

    // The prologue binds the plaintext preamble: if the two sides saw different preamble
    // bytes (an in-path attacker rewrote the version or mode exchange), the handshake must
    // fail even though the passwords match.
    func testPrologueMismatchFails() throws {
        let psk = derivePsk("same password")
        let good = buildPrologue(
            initiatorTranscript: hex("000000000000000a0000000000000001"),
            responderTranscript: hex("000000000000000a0000000000000000")
        )
        var tamperedResp = hex("000000000000000a0000000000000000")
        tamperedResp[15] ^= 0x01
        let tampered = buildPrologue(
            initiatorTranscript: hex("000000000000000a0000000000000001"),
            responderTranscript: tamperedResp
        )
        let initHS = try NoiseHandshakeState(role: .initiator, psk: psk, prologue: good)
        let respHS = try NoiseHandshakeState(role: .responder, psk: psk, prologue: tampered)
        XCTAssertThrowsError(try respHS.readMessage(initHS.writeMessage()))
    }
}

// Cross-platform verification of the discovery announcement wire format (Discovery.swift).
// The 93-byte layout and its HMAC were pinned only between Rust (core/src/discovery.rs,
// test_cross_platform_vector) and Kotlin (DiscoveryUnitTest.crossPlatformVector); this class
// holds Swift to the same bytes. Discovery is what shared network mode uses to find a peer,
// and it is the one part of the v10 wire protocol Apple could have drifted on silently:
// Apple<->Apple is exactly the pair that *requires* shared network, so a Swift-only change to
// a field width, an endianness, or the signed byte range would break the pair least likely to
// be caught by testing against Rust or Android.
final class DiscoveryTests: XCTestCase {

    // Byte-for-byte identical to the Rust and Kotlin vectors. Receiver role, 192.168.1.42,
    // port 3290, timestamp 1750000000, sequence 7, nonce of 32 0x11s, signed with the key
    // 0x00,0x01,...,0x1f.
    private let vectorHex = "4643415000010100000000c0a8012a0cda00000000684ee180000000071111111111111111111111111111111111111111111111111111111111111111adc75d44854c84be1627ef8933f16d0fcb26807fccb7562ea3609f13982f7a9a"

    private let vectorKey: [UInt8] = (0..<32).map { UInt8($0) }

    private func hex(_ s: String) -> Data {
        var d = Data()
        var i = s.startIndex
        while i < s.endIndex {
            let j = s.index(i, offsetBy: 2)
            d.append(UInt8(s[i..<j], radix: 16)!)
            i = j
        }
        return d
    }

    private func toHex(_ d: Data) -> String {
        d.map { String(format: "%02x", $0) }.joined()
    }

    // Serialization: build the announcement the vector describes and assert the exact wire
    // bytes, including the HMAC. The initializer stamps the current time and a random nonce,
    // so both are overwritten here -- everything else is what the app itself would produce.
    func testAnnouncementCrossPlatformVector() {
        var announcement = DiscoveryAnnouncement(
            role: .receiver,
            ipAddress: "192.168.1.42",
            sequence: 7
        )
        announcement.timestamp = 1_750_000_000
        announcement.nonce = [UInt8](repeating: 0x11, count: 32)
        announcement.sign(key: vectorKey)

        XCTAssertEqual(toHex(announcement.serialize()), vectorHex)
        XCTAssertTrue(announcement.verify(key: vectorKey))
    }

    // Deserialization, against the same foreign bytes: a Swift receiver has to read what a
    // Rust or Kotlin sender wrote. Field-by-field, because a compensating pair of offset
    // errors could still round-trip cleanly through serialize().
    func testAnnouncementDeserializesCrossPlatformVector() throws {
        let parsed = try XCTUnwrap(DiscoveryAnnouncement.deserialize(hex(vectorHex)))

        XCTAssertEqual(parsed.magic, DISCOVERY_MAGIC)
        XCTAssertEqual(parsed.version, 1)
        XCTAssertEqual(parsed.role, .receiver)
        XCTAssertEqual(parsed.capabilities, 0)
        XCTAssertEqual(parsed.getIPString(), "192.168.1.42")
        XCTAssertEqual(parsed.port, DISCOVERY_PORT)
        XCTAssertEqual(parsed.timestamp, 1_750_000_000)
        XCTAssertEqual(parsed.sequence, 7)
        XCTAssertEqual(parsed.nonce, [UInt8](repeating: 0x11, count: 32))

        // The HMAC covers the first 61 bytes, so a peer's signature must verify here.
        XCTAssertTrue(parsed.verify(key: vectorKey))
        XCTAssertFalse(parsed.verify(key: [UInt8](repeating: 0xff, count: 32)))

        XCTAssertEqual(toHex(parsed.serialize()), vectorHex)
    }

    // A single flipped bit anywhere in the signed range must fail verification -- this is what
    // keeps an unauthenticated announcement from steering a transfer to an attacker's address.
    func testTamperedAnnouncementFailsVerification() throws {
        var bytes = Array(hex(vectorHex))
        bytes[12] ^= 0x01  // second octet of the IP address (offset 11..14), inside the signed prefix
        let parsed = try XCTUnwrap(DiscoveryAnnouncement.deserialize(Data(bytes)))
        XCTAssertEqual(parsed.getIPString(), "192.169.1.42")
        XCTAssertFalse(parsed.verify(key: vectorKey))
    }
}

// safeDestinationURL resolves a peer-supplied filename to a path guaranteed to stay inside
// the chosen receive folder. Covers the /private firmlink regression (bare filenames were
// rejected as unsafe) and the directory-traversal cases the check exists to block.
final class SafeDestinationURLTests: XCTestCase {

    // A bare filename lands directly inside the base directory.
    func testBareFilename() throws {
        let base = URL(fileURLWithPath: "/tmp", isDirectory: true)
        let dest = try safeDestinationURL(baseDir: base, filename: "2452_53122.jpg")
        XCTAssertEqual(dest.path, "/tmp/2452_53122.jpg")
    }

    // Regression: iOS document-picker folders arrive in /private firmlink form
    // (/private/var/mobile/...). standardizedFileURL collapses the existing base to /var...
    // but leaves a not-yet-created appended file as /private/var..., so the containment
    // check used to throw UnsafeFilename on a perfectly safe name. It must not.
    func testPrivateFirmlinkBaseAcceptsBareFilename() throws {
        let base = URL(fileURLWithPath: "/private/tmp", isDirectory: true)
        XCTAssertNoThrow(try safeDestinationURL(baseDir: base, filename: "photo.jpg"))
        let dest = try safeDestinationURL(baseDir: base, filename: "photo.jpg")
        XCTAssertEqual(dest.path, "/tmp/photo.jpg")
    }

    // Folder transfers legitimately carry subdirectories.
    func testNestedRelativePathStaysInside() throws {
        let base = URL(fileURLWithPath: "/private/tmp", isDirectory: true)
        let dest = try safeDestinationURL(baseDir: base, filename: "album/2024/a.jpg")
        XCTAssertEqual(dest.path, "/tmp/album/2024/a.jpg")
    }

    // A leading slash must not escape to the filesystem root: empty components collapse and
    // the file lands inside the base ("/etc/passwd" -> <base>/etc/passwd).
    func testLeadingSlashLandsInsideBase() throws {
        let base = URL(fileURLWithPath: "/private/tmp", isDirectory: true)
        let dest = try safeDestinationURL(baseDir: base, filename: "/etc/passwd")
        XCTAssertEqual(dest.path, "/tmp/etc/passwd")
    }

    // Parent-directory traversal must be rejected, whether leading or nested.
    func testRejectsParentTraversal() {
        let base = URL(fileURLWithPath: "/private/tmp", isDirectory: true)
        XCTAssertThrowsError(try safeDestinationURL(baseDir: base, filename: "../escape.jpg"))
        XCTAssertThrowsError(try safeDestinationURL(baseDir: base, filename: "a/../../escape.jpg"))
    }

    // A name with no usable components must be rejected, not silently written to the base.
    func testRejectsEmptyAndDotOnlyNames() {
        let base = URL(fileURLWithPath: "/private/tmp", isDirectory: true)
        XCTAssertThrowsError(try safeDestinationURL(baseDir: base, filename: ""))
        XCTAssertThrowsError(try safeDestinationURL(baseDir: base, filename: "."))
        XCTAssertThrowsError(try safeDestinationURL(baseDir: base, filename: "./"))
    }
}

// hashFile is handed the same FileHandle the sender is about to read the file body from
// (Send.swift), so it must hash the whole file no matter where the handle is sitting and
// must leave the offset where it found it.
final class HashFileTests: XCTestCase {

    private var fileURL: URL!

    override func setUpWithError() throws {
        fileURL = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("hashfile-\(UUID().uuidString).bin")
        // bigger than hashFile's 10MB read so the multi-read path is covered
        var contents = Data(count: 25_000_000)
        contents.replaceSubrange(0..<4, with: Data([0xde, 0xad, 0xbe, 0xef]))
        try contents.write(to: fileURL)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: fileURL)
    }

    // Regression: hashing consumed the handle, so the sender's first read of the file body
    // hit EOF and returned nil. That only happened when the receiver had a file of the same
    // name and size whose hash didn't match — i.e. exactly the case where the send must go
    // ahead — and the nil unwrap in the send loop crashed the app.
    func testLeavesOffsetWhereItFoundIt() throws {
        let handle = try FileHandle(forReadingFrom: fileURL)
        defer { try? handle.close() }

        let _ = try hashFile(file: handle)
        XCTAssertEqual(try handle.offset(), 0)
        XCTAssertEqual(try handle.read(upToCount: 4), Data([0xde, 0xad, 0xbe, 0xef]))

        try handle.seek(toOffset: 1_000)
        let _ = try hashFile(file: handle)
        XCTAssertEqual(try handle.offset(), 1_000)
    }

    // A non-zero starting offset must not change the digest: both ends hash the whole file.
    func testHashesWholeFileFromAnyOffset() throws {
        let handle = try FileHandle(forReadingFrom: fileURL)
        defer { try? handle.close() }

        let expected = SHA256.hash(data: try Data(contentsOf: fileURL))
        XCTAssertEqual(try hashFile(file: handle), expected)
        try handle.seekToEnd()
        XCTAssertEqual(try hashFile(file: handle), expected)
    }
}
