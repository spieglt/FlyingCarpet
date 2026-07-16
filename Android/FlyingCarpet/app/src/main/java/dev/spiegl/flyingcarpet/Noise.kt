package dev.spiegl.flyingcarpet

import com.southernstorm.noise.crypto.Curve25519
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.io.OutputStream
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.Mac
import javax.crypto.spec.IvParameterSpec
import javax.crypto.spec.SecretKeySpec

// v10+ encrypted transport for all transfers (both hotspot and shared network mode). A
// hand-rolled implementation of the Noise NNpsk0 handshake and ChaChaPoly transport,
// mirroring the Rust reference in core/src/noise.rs; see docs/shared-network-crypto.md.
//
// Hand-rolled (rather than a Noise library) because the app targets minSdk 29 / Java 8 and
// must interoperate with the modern Noise_NNpsk0 the Rust core speaks: rweather/noise-java
// implements the deprecated pre-2018 PSK scheme, and jchambers/java-noise needs Java 17 and
// API 30. Only X25519 isn't in the API-29 platform crypto, so it comes from the vendored
// pure-Java com.southernstorm.noise.crypto.Curve25519; everything else is JCA. The whole
// thing is verified byte-for-byte against the official cacophony vector in NoiseUnitTest.

const val NOISE_PROTOCOL_NAME = "Noise_NNpsk0_25519_ChaChaPoly_SHA256"
// Must be byte-identical across Rust, Swift, and Kotlin.
val PSK_SALT: ByteArray = "Flying Carpet v10 shared network PSK".toByteArray(Charsets.UTF_8)
const val PBKDF2_ITERS = 600_000
// Domain-separation label for the discovery announcement HMAC key (derived from the PSK,
// never from a fast hash of the password — see deriveDiscoveryKey).
val DISCOVERY_INFO: ByteArray = "Flying Carpet v10 discovery".toByteArray(Charsets.UTF_8)

private const val DHLEN = 32
private const val TAGLEN = 16
private const val NOISE_MAX_MESSAGE = 65535
private const val NOISE_MAX_PLAINTEXT = NOISE_MAX_MESSAGE - TAGLEN // 65519

enum class NoiseRole { INITIATOR, RESPONDER }

// ---- primitives ----

private fun sha256(vararg parts: ByteArray): ByteArray {
    val md = MessageDigest.getInstance("SHA-256")
    for (p in parts) md.update(p)
    return md.digest()
}

private fun hmacSha256(key: ByteArray, data: ByteArray): ByteArray {
    val mac = Mac.getInstance("HmacSHA256")
    mac.init(SecretKeySpec(key, "HmacSHA256"))
    return mac.doFinal(data)
}

// Noise HKDF: returns `num` (2 or 3) 32-byte outputs derived from the chaining key and IKM.
private fun hkdf(chainingKey: ByteArray, ikm: ByteArray, num: Int): Array<ByteArray> {
    val tempKey = hmacSha256(chainingKey, ikm)
    val o1 = hmacSha256(tempKey, byteArrayOf(1))
    val o2 = hmacSha256(tempKey, o1 + byteArrayOf(2))
    if (num == 2) return arrayOf(o1, o2)
    val o3 = hmacSha256(tempKey, o2 + byteArrayOf(3))
    return arrayOf(o1, o2, o3)
}

// X25519. With peerPublic == null, derives the public key from the private (base point).
private fun x25519(privateKey: ByteArray, peerPublic: ByteArray?): ByteArray {
    val out = ByteArray(DHLEN)
    Curve25519.eval(out, 0, privateKey, peerPublic)
    return out
}

// Builds the canonical Noise prologue from the plaintext preamble transcript.
// initiatorTranscript is every byte the Noise initiator sent during the preamble (version +
// mode exchange) and responderTranscript every byte the responder sent; each is
// length-prefixed (u64 big-endian, matching the app's length idiom) so the encoding is
// unambiguous. Each side computes this from its own sent/received bytes — the initiator as
// (sent, received), the responder as (received, sent) — so any in-flight tampering with the
// preamble makes the prologues differ, which fails the handshake. Must be byte-identical
// across Rust, Swift, and Kotlin (see core/src/noise.rs build_prologue).
fun buildPrologue(initiatorTranscript: ByteArray, responderTranscript: ByteArray): ByteArray {
    fun be64(n: Long): ByteArray {
        val b = ByteArray(8)
        for (i in 0 until 8) b[i] = ((n ushr (8 * (7 - i))) and 0xff).toByte()
        return b
    }
    return be64(initiatorTranscript.size.toLong()) + initiatorTranscript +
        be64(responderTranscript.size.toLong()) + responderTranscript
}

// Wrap the raw socket streams during the plaintext preamble, recording every byte sent and
// received so the transcript can be bound into the Noise prologue (see buildPrologue).
// Recording at the stream boundary — rather than inside the preamble functions — means no
// exchanged byte can be missed, whatever branch the version/mode negotiation takes.
class RecordingInputStream(val inner: InputStream) : InputStream() {
    private val recorded = ByteArrayOutputStream()

    fun transcript(): ByteArray = recorded.toByteArray()

    override fun read(): Int {
        val b = inner.read()
        if (b >= 0) recorded.write(b)
        return b
    }

    override fun read(b: ByteArray, off: Int, len: Int): Int {
        val n = inner.read(b, off, len)
        if (n > 0) recorded.write(b, off, n)
        return n
    }

    override fun close() = inner.close()
}

class RecordingOutputStream(val inner: OutputStream) : OutputStream() {
    private val recorded = ByteArrayOutputStream()

    fun transcript(): ByteArray = recorded.toByteArray()

    override fun write(b: Int) {
        inner.write(b)
        recorded.write(b)
    }

    override fun write(b: ByteArray, off: Int, len: Int) {
        inner.write(b, off, len)
        recorded.write(b, off, len)
    }

    override fun flush() = inner.flush()
    override fun close() = inner.close()
}

// Derives the 32-byte Noise pre-shared key from the transfer password with
// PBKDF2-HMAC-SHA256 over the UTF-8 password bytes. Implemented on JCE's HMAC (rather than
// PBKDF2WithHmacSHA256 / PBEKeySpec) so the encoding is unambiguously UTF-8 and matches the
// Rust `pbkdf2_hmac::<Sha256>` output exactly.
fun derivePsk(password: String): ByteArray {
    val mac = Mac.getInstance("HmacSHA256")
    mac.init(SecretKeySpec(password.toByteArray(Charsets.UTF_8), "HmacSHA256"))
    // dkLen == hLen == 32, so a single block.
    mac.update(PSK_SALT)
    var u = mac.doFinal(byteArrayOf(0, 0, 0, 1)) // U1 = HMAC(pw, salt || INT(1))
    val t = u.copyOf()
    for (i in 2..PBKDF2_ITERS) {
        u = mac.doFinal(u)
        for (k in t.indices) t[k] = (t[k].toInt() xor u[k].toInt()).toByte()
    }
    return t
}

// Derives the discovery announcement HMAC key from the PBKDF2-stretched PSK, with a fixed
// label for domain separation (the Noise PSK itself is never used outside the handshake).
// Keying discovery from the stretched PSK — instead of the old SHA256(password) — means a
// captured announcement costs an offline attacker 600k PBKDF2 iterations per password
// guess, the same as the handshake, so the password can't be cracked while it's still
// live. Must be byte-identical across Rust, Swift, and Kotlin.
fun deriveDiscoveryKey(psk: ByteArray): ByteArray = hmacSha256(psk, DISCOVERY_INFO)

// ---- transport cipher (ChaCha20-Poly1305 with the Noise nonce) ----

class NoiseCipherState(private val key: ByteArray) {
    private var nonce = 0L
    private val cipher = Cipher.getInstance("ChaCha20-Poly1305")

    // Noise 96-bit nonce: 32 bits of zero followed by the 64-bit little-endian counter.
    private fun iv(): IvParameterSpec {
        val v = ByteArray(12)
        for (i in 0 until 8) v[4 + i] = ((nonce ushr (8 * i)) and 0xff).toByte()
        return IvParameterSpec(v)
    }

    fun encryptWithAd(ad: ByteArray, plaintext: ByteArray): ByteArray {
        cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "ChaCha20"), iv())
        if (ad.isNotEmpty()) cipher.updateAAD(ad)
        val out = cipher.doFinal(plaintext)
        nonce++
        return out
    }

    fun decryptWithAd(ad: ByteArray, ciphertext: ByteArray): ByteArray {
        cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(key, "ChaCha20"), iv())
        if (ad.isNotEmpty()) cipher.updateAAD(ad)
        val out = cipher.doFinal(ciphertext) // throws AEADBadTagException on a bad tag
        nonce++
        return out
    }
}

// ---- Noise symmetric state ----

private class SymmetricState {
    var ck: ByteArray
    var h: ByteArray
    private var cs: NoiseCipherState? = null

    init {
        val name = NOISE_PROTOCOL_NAME.toByteArray(Charsets.UTF_8)
        h = if (name.size <= 32) name.copyOf(32) else sha256(name)
        ck = h.copyOf()
    }

    fun mixHash(data: ByteArray) {
        h = sha256(h, data)
    }

    fun mixKey(input: ByteArray) {
        val out = hkdf(ck, input, 2)
        ck = out[0]
        cs = NoiseCipherState(out[1])
    }

    fun mixKeyAndHash(input: ByteArray) {
        val out = hkdf(ck, input, 3)
        ck = out[0]
        mixHash(out[1])
        cs = NoiseCipherState(out[2])
    }

    fun encryptAndHash(plaintext: ByteArray): ByteArray {
        val ct = cs?.encryptWithAd(h, plaintext) ?: plaintext
        mixHash(ct)
        return ct
    }

    fun decryptAndHash(ciphertext: ByteArray): ByteArray {
        val pt = cs?.decryptWithAd(h, ciphertext) ?: ciphertext
        mixHash(ciphertext)
        return pt
    }

    fun split(): Pair<NoiseCipherState, NoiseCipherState> {
        val out = hkdf(ck, ByteArray(0), 2)
        return Pair(NoiseCipherState(out[0]), NoiseCipherState(out[1]))
    }
}

// ---- NNpsk0 handshake ----
//
// Messages: `-> psk, e` then `<- e, ee`. In PSK mode the `e` token additionally MixKey's the
// ephemeral public key. Testable independently of sockets: NoiseUnitTest drives this with
// fixed ephemerals to check message bytes against the official vector.
class NoiseHandshakeState(
    private val role: NoiseRole,
    private val psk: ByteArray,
    prologue: ByteArray = ByteArray(0),
    private val fixedEphemeral: ByteArray? = null,
) {
    private val ss = SymmetricState()
    private val ePriv = fixedEphemeral ?: randomScalar()
    private val ePub = x25519(ePriv, null)
    private var rePub: ByteArray? = null
    private var messageIndex = 0

    init {
        ss.mixHash(prologue)
    }

    private fun randomScalar(): ByteArray {
        val s = ByteArray(DHLEN)
        SecureRandom().nextBytes(s)
        return s
    }

    // Writes the next outgoing handshake message (with an optional payload).
    fun writeMessage(payload: ByteArray = ByteArray(0)): ByteArray {
        val out = ByteArrayOutputStream()
        if (messageIndex == 0) {
            // -> psk, e
            ss.mixKeyAndHash(psk)
            out.write(ePub)
            ss.mixHash(ePub)
            ss.mixKey(ePub)
        } else {
            // <- e, ee
            out.write(ePub)
            ss.mixHash(ePub)
            ss.mixKey(ePub)
            ss.mixKey(x25519(ePriv, rePub!!))
        }
        out.write(ss.encryptAndHash(payload))
        messageIndex++
        return out.toByteArray()
    }

    // Reads the next incoming handshake message and returns its payload. Throws if the
    // authenticated payload doesn't decrypt (e.g. mismatched password).
    fun readMessage(message: ByteArray): ByteArray {
        val re = message.copyOfRange(0, DHLEN)
        if (messageIndex == 0) {
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
            ss.mixKey(x25519(ePriv, re))
        }
        val payload = ss.decryptAndHash(message.copyOfRange(DHLEN, message.size))
        messageIndex++
        return payload
    }

    fun handshakeHash(): ByteArray = ss.h

    // Returns (send cipher, receive cipher) oriented for this party's role.
    fun split(): Pair<NoiseCipherState, NoiseCipherState> {
        val (c1, c2) = ss.split()
        return if (role == NoiseRole.INITIATOR) Pair(c1, c2) else Pair(c2, c1)
    }
}

class NoiseTransport(val input: NoiseInputStream, val output: NoiseOutputStream)

// Runs the handshake over the raw socket streams and returns encrypting/decrypting stream
// wrappers. The initiator (TCP client) sends the first message. `prologue` binds the
// plaintext preamble transcript (see buildPrologue): if either the password or the
// preamble bytes differ between the peers, the handshake fails.
fun noiseHandshake(
    rawIn: InputStream,
    rawOut: OutputStream,
    role: NoiseRole,
    psk: ByteArray,
    prologue: ByteArray,
): NoiseTransport {
    val hs = NoiseHandshakeState(role, psk, prologue)
    try {
        if (role == NoiseRole.INITIATOR) {
            writeFrame(rawOut, hs.writeMessage())
            rawOut.flush()
            hs.readMessage(readFrame(rawIn))
        } else {
            hs.readMessage(readFrame(rawIn))
            writeFrame(rawOut, hs.writeMessage())
            rawOut.flush()
        }
    } catch (e: javax.crypto.AEADBadTagException) {
        throw Exception(
            "Could not establish a secure connection. Check that the password matches on both devices. (This can also mean the connection was tampered with.)"
        )
    }
    val (send, recv) = hs.split()
    return NoiseTransport(NoiseInputStream(rawIn, recv), NoiseOutputStream(rawOut, send))
}

// Each Noise message (handshake or transport record) is prefixed with its length as a
// 2-byte big-endian integer; 2 bytes because Noise caps a message at 65535 bytes.
private fun writeFrame(out: OutputStream, msg: ByteArray) {
    val frame = ByteArray(2 + msg.size)
    frame[0] = ((msg.size ushr 8) and 0xff).toByte()
    frame[1] = (msg.size and 0xff).toByte()
    System.arraycopy(msg, 0, frame, 2, msg.size)
    out.write(frame)
}

private fun readFrame(input: InputStream): ByteArray {
    val hi = input.read()
    val lo = input.read()
    if (hi < 0 || lo < 0) throw Exception("Peer connection closed during Noise handshake.")
    val len = (hi shl 8) or lo
    val buf = ByteArray(len)
    var read = 0
    while (read < len) {
        val n = input.read(buf, read, len - read)
        if (n < 0) throw Exception("Peer connection closed during Noise handshake.")
        read += n
    }
    return buf
}

// Encrypts writes into length-prefixed Noise transport records, splitting anything larger
// than one Noise message across multiple records. Wraps the raw socket OutputStream so the
// existing transfer code writes to it exactly like the plain socket stream.
class NoiseOutputStream(private val out: OutputStream, private val cipher: NoiseCipherState) :
    OutputStream() {
    private val emptyAd = ByteArray(0)

    override fun write(b: Int) {
        write(byteArrayOf(b.toByte()), 0, 1)
    }

    override fun write(b: ByteArray, off: Int, len: Int) {
        var pos = off
        var remaining = len
        while (remaining > 0) {
            val take = minOf(remaining, NOISE_MAX_PLAINTEXT)
            val record = cipher.encryptWithAd(emptyAd, b.copyOfRange(pos, pos + take))
            writeFrame(out, record)
            pos += take
            remaining -= take
        }
    }

    override fun flush() = out.flush()
    override fun close() = out.close()
}

// Decrypts length-prefixed Noise transport records into a plaintext byte stream. Wraps the
// raw socket InputStream so the existing transfer code (readNBytes) reads from it exactly
// like the plain socket stream.
class NoiseInputStream(private val input: InputStream, private val cipher: NoiseCipherState) :
    InputStream() {
    private val emptyAd = ByteArray(0)
    private var plain = ByteArray(0)
    private var plainPos = 0

    override fun read(): Int {
        val one = ByteArray(1)
        val n = read(one, 0, 1)
        return if (n < 0) -1 else one[0].toInt() and 0xff
    }

    override fun read(b: ByteArray, off: Int, len: Int): Int {
        if (len == 0) return 0
        if (plainPos >= plain.size && !fill()) return -1
        val n = minOf(plain.size - plainPos, len)
        System.arraycopy(plain, plainPos, b, off, n)
        plainPos += n
        return n
    }

    private fun fill(): Boolean {
        val hi = input.read()
        if (hi < 0) return false // EOF between records
        val lo = input.read()
        if (lo < 0) throw Exception("Peer connection closed mid-record.")
        val recordLen = (hi shl 8) or lo
        if (recordLen < TAGLEN || recordLen > NOISE_MAX_MESSAGE) {
            throw Exception("Invalid Noise record length: $recordLen")
        }
        val record = ByteArray(recordLen)
        var read = 0
        while (read < recordLen) {
            val n = input.read(record, read, recordLen - read)
            if (n < 0) throw Exception("Peer connection closed mid-record.")
            read += n
        }
        plain = cipher.decryptWithAd(emptyAd, record)
        plainPos = 0
        return true
    }
}
