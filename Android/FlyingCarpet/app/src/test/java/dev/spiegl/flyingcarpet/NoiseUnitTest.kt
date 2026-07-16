package dev.spiegl.flyingcarpet

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream

class NoiseUnitTest {

    private fun hex(s: String): ByteArray =
        ByteArray(s.length / 2) {
            ((s[it * 2].digitToInt(16) shl 4) or s[it * 2 + 1].digitToInt(16)).toByte()
        }

    private fun toHex(b: ByteArray): String = b.joinToString("") { "%02x".format(it) }

    // Spec conformance: drive the hand-rolled NNpsk0 with the official cacophony test vector
    // for Noise_NNpsk0_25519_ChaChaPoly_SHA256 and assert the handshake message ciphertexts,
    // the handshake hash, and the first transport record match byte-for-byte. This is the
    // same vector asserted by the Rust reference (core/src/noise.rs, official_noise_test_vector),
    // so passing it proves Android interoperates with Rust.
    @Test
    fun officialNoiseTestVector() {
        val prologue = hex("4a6f686e2047616c74") // "John Galt"
        val psk = hex("54686973206973206d7920417573747269616e20706572737065637469766521")
        val initEph = hex("893e28b9dc6ca8d611ab664754b8ceb7bac5117349a4439a6b0569da977c464a")
        val respEph = hex("bbdb4cdbd309f1a1f2e1456967fe288cadd6f712d65dc7b7793d5e63da6b375b")

        val init = NoiseHandshakeState(NoiseRole.INITIATOR, psk, prologue, initEph)
        val resp = NoiseHandshakeState(NoiseRole.RESPONDER, psk, prologue, respEph)

        // message 0: initiator -> responder ("Ludwig von Mises")
        val p0 = hex("4c756477696720766f6e204d69736573")
        val msg0 = init.writeMessage(p0)
        assertEquals(
            "ca35def5ae56cec33dc2036731ab14896bc4c75dbb07a61f879f8e3afa4c794479b962b8aff8485742ac32f905ba45369e2465fb59e138a93d67a0d1266b6a54",
            toHex(msg0),
        )
        assertArrayEquals(p0, resp.readMessage(msg0))

        // message 1: responder -> initiator ("Murray Rothbard")
        val p1 = hex("4d757272617920526f746862617264")
        val msg1 = resp.writeMessage(p1)
        assertEquals(
            "95ebc60d2b1fa672c1f46a8aa265ef51bfe38e7ccb39ec5be34069f144808843d6062704d5a9c422a8e834423f8c1feada7e8d0d910a1a2cd030fb584221e3",
            toHex(msg1),
        )
        assertArrayEquals(p1, init.readMessage(msg1))

        // handshake hash matches the spec value
        val expectedHh = "f4d03dc34495c95729ea6de9e1b59004b59733102488b3e24bc441e0be208eaf"
        assertEquals(expectedHh, toHex(init.handshakeHash()))
        assertEquals(expectedHh, toHex(resp.handshakeHash()))

        // first transport message: initiator -> responder ("F. A. Hayek")
        val (initSend, _) = init.split()
        val (_, respRecv) = resp.split()
        val p2 = hex("462e20412e20486179656b")
        val record = initSend.encryptWithAd(ByteArray(0), p2)
        assertEquals("e632c3763d7669067383433197a3baddf146e9e70ad4b4e9e59e0f", toHex(record))
        assertArrayEquals(p2, respRecv.decryptWithAd(ByteArray(0), record))
    }

    // Cross-platform known-answer for the PSK derivation (must match Rust's derive_psk).
    @Test
    fun pskKnownAnswer() {
        assertEquals(
            "a3d8b7f17f2252e4c2847a365ab2f392beaa996b7e51dd6fa19ff1ad08938619",
            toHex(derivePsk("flyingcarpet")),
        )
    }

    // Cross-platform known-answer for the discovery HMAC key (must match Rust's
    // derive_discovery_key): HMAC-SHA256 keyed by the PSK over the DISCOVERY_INFO label.
    @Test
    fun discoveryKeyKnownAnswer() {
        assertEquals(
            "45e49b632788b21069bf48720d6af230ecbd936b3cb16c898a8e1eac51944112",
            toHex(deriveDiscoveryKey(derivePsk("flyingcarpet"))),
        )
    }

    // App-parameter known-answer (empty prologue, fixed PSK/ephemerals) matching the Rust
    // reference's handshake_known_answer (docs §9): confirms our exact usage produces
    // identical bytes across implementations.
    @Test
    fun appHandshakeKnownAnswer() {
        val psk = ByteArray(32) { 0x2a }
        val initEph = ByteArray(32) { 0x01 }
        val respEph = ByteArray(32) { 0x02 }

        val init = NoiseHandshakeState(NoiseRole.INITIATOR, psk, ByteArray(0), initEph)
        val resp = NoiseHandshakeState(NoiseRole.RESPONDER, psk, ByteArray(0), respEph)

        val msg1 = init.writeMessage()
        assertEquals(
            "a4e09292b651c278b9772c569f5fa9bb13d906b46ab68c9df9dc2b4409f8a209a3e9c18456aba2185de800ffaca55b22",
            toHex(msg1),
        )
        resp.readMessage(msg1)

        val msg2 = resp.writeMessage()
        assertEquals(
            "ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59d887595caf8a0b110dfab84e6b41eafc",
            toHex(msg2),
        )
        init.readMessage(msg2)

        val (initSend, _) = init.split()
        val record = initSend.encryptWithAd(ByteArray(0), "hello flying carpet".toByteArray())
        assertEquals(
            "124a00c03b4544f746828bbf9ae2d8d595a9ac1fea988f43f7206c3880180b954f9147",
            toHex(record),
        )
    }

    private fun handshakeInMemory(password: String): Pair<NoiseCipherState, NoiseCipherState> {
        val psk = derivePsk(password)
        val init = NoiseHandshakeState(NoiseRole.INITIATOR, psk)
        val resp = NoiseHandshakeState(NoiseRole.RESPONDER, psk)
        resp.readMessage(init.writeMessage())
        init.readMessage(resp.writeMessage())
        val (initSend, _) = init.split()
        val (_, respRecv) = resp.split()
        return Pair(initSend, respRecv)
    }

    // Exercises NoiseOutputStream/NoiseInputStream framing with a payload larger than one
    // Noise record (so it spans several records), end to end.
    @Test
    fun streamRoundTripMultiRecord() {
        val (initSend, respRecv) = handshakeInMemory("correct horse")
        val big = ByteArray(200_000) { (it % 251).toByte() }

        val wire = ByteArrayOutputStream()
        val nout = NoiseOutputStream(wire, initSend)
        nout.write(big)
        nout.flush()

        val nin = NoiseInputStream(ByteArrayInputStream(wire.toByteArray()), respRecv)
        val got = ByteArray(big.size)
        var read = 0
        while (read < got.size) {
            val n = nin.read(got, read, got.size - read)
            if (n < 0) break
            read += n
        }
        assertEquals(big.size, read)
        assertArrayEquals(big, got)
    }

    // A mismatched password makes the responder's read of the first message fail, so the
    // handshake cannot complete.
    @Test
    fun wrongPasswordFailsHandshake() {
        val init = NoiseHandshakeState(NoiseRole.INITIATOR, derivePsk("password-one"))
        val resp = NoiseHandshakeState(NoiseRole.RESPONDER, derivePsk("password-two"))
        val msg1 = init.writeMessage()
        try {
            resp.readMessage(msg1)
            fail("responder should have rejected the message under a mismatched PSK")
        } catch (e: Exception) {
            // expected: bad tag / decryption failure
        }
    }

    // Corrupting any single byte of a transport record must make the receiver's decrypt
    // fail (the Poly1305 tag no longer verifies).
    @Test
    fun tamperingIsDetected() {
        val (initSend, respRecv) = handshakeInMemory("shared secret")

        // first record, untampered: decrypts fine (sanity-checks the setup)
        val ok = initSend.encryptWithAd(ByteArray(0), "authentic payload".toByteArray())
        assertArrayEquals(
            "authentic payload".toByteArray(),
            respRecv.decryptWithAd(ByteArray(0), ok),
        )

        // second record with one bit flipped in the ciphertext: must fail to authenticate
        val record = initSend.encryptWithAd(ByteArray(0), "authentic payload".toByteArray())
        record[record.size / 2] = (record[record.size / 2].toInt() xor 0x01).toByte()
        try {
            respRecv.decryptWithAd(ByteArray(0), record)
            fail("tampered record should have failed authentication")
        } catch (e: Exception) {
            // expected: AEADBadTagException
        }
    }

    // Same guarantee for a corrupted handshake message: the responder must reject it.
    @Test
    fun tamperedHandshakeIsDetected() {
        val psk = derivePsk("shared secret")
        val init = NoiseHandshakeState(NoiseRole.INITIATOR, psk)
        val resp = NoiseHandshakeState(NoiseRole.RESPONDER, psk)
        val msg1 = init.writeMessage()
        // corrupt the encrypted payload section (past the 32-byte ephemeral)
        msg1[msg1.size - 1] = (msg1[msg1.size - 1].toInt() xor 0x01).toByte()
        try {
            resp.readMessage(msg1)
            fail("tampered handshake message should have failed authentication")
        } catch (e: Exception) {
            // expected: bad tag / decryption failure
        }
    }

    // Cross-platform known-answer for the prologue-bound handshake, matching the Rust
    // reference's prologue_known_answer (docs §9): the app-style preamble transcript
    // (version 10 + mode, 8-byte big-endian each) framed by buildPrologue, with fixed
    // PSK/ephemerals. The transport record intentionally matches appHandshakeKnownAnswer's:
    // per the Noise spec the prologue only enters h (gating the handshake MACs), never the
    // chaining key — asserted anyway so a port that wrongly mixes it into ck fails here.
    @Test
    fun prologueKnownAnswer() {
        val initTranscript = hex("000000000000000a0000000000000001")
        val respTranscript = hex("000000000000000a0000000000000000")
        val prologue = buildPrologue(initTranscript, respTranscript)
        assertEquals(
            "0000000000000010000000000000000a00000000000000010000000000000010000000000000000a0000000000000000",
            toHex(prologue),
        )

        val psk = ByteArray(32) { 0x2a }
        val init = NoiseHandshakeState(NoiseRole.INITIATOR, psk, prologue, ByteArray(32) { 0x01 })
        val resp = NoiseHandshakeState(NoiseRole.RESPONDER, psk, prologue, ByteArray(32) { 0x02 })

        val msg1 = init.writeMessage()
        assertEquals(
            "a4e09292b651c278b9772c569f5fa9bb13d906b46ab68c9df9dc2b4409f8a2093ae03dc8524f79ac9696d6c155df9a3c",
            toHex(msg1),
        )
        resp.readMessage(msg1)

        val msg2 = resp.writeMessage()
        assertEquals(
            "ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59d2668070263116ce557500fbe3fd3ba4",
            toHex(msg2),
        )
        init.readMessage(msg2)

        val (initSend, _) = init.split()
        val record = initSend.encryptWithAd(ByteArray(0), "hello flying carpet".toByteArray())
        assertEquals(
            "124a00c03b4544f746828bbf9ae2d8d595a9ac1fea988f43f7206c3880180b954f9147",
            toHex(record),
        )
    }

    // The prologue binds the plaintext preamble: if the two sides saw different preamble
    // bytes (an in-path attacker rewrote the version or mode exchange), the handshake must
    // fail even though the passwords match.
    @Test
    fun prologueMismatchFailsHandshake() {
        val psk = derivePsk("same password")
        val good = buildPrologue(
            hex("000000000000000a0000000000000001"),
            hex("000000000000000a0000000000000000"),
        )
        val tamperedResp = hex("000000000000000a0000000000000000")
        tamperedResp[15] = (tamperedResp[15].toInt() xor 0x01).toByte()
        val tampered = buildPrologue(hex("000000000000000a0000000000000001"), tamperedResp)

        val init = NoiseHandshakeState(NoiseRole.INITIATOR, psk, good)
        val resp = NoiseHandshakeState(NoiseRole.RESPONDER, psk, tampered)
        try {
            resp.readMessage(init.writeMessage())
            fail("responder should have rejected the message under a mismatched prologue")
        } catch (e: Exception) {
            // expected: bad tag / decryption failure
        }
    }
}
