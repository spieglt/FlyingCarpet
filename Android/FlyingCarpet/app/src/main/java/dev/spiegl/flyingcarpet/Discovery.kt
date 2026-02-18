package dev.spiegl.flyingcarpet

import android.content.Context
import android.net.wifi.WifiManager
import android.util.Log
import kotlinx.coroutines.*
import java.net.*
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean

const val MULTICAST_ADDR = "239.255.73.67"
const val DISCOVERY_PORT = 3290
val DISCOVERY_MAGIC = byteArrayOf('F'.code.toByte(), 'C'.code.toByte(), 'A'.code.toByte(), 'P'.code.toByte())
const val ANNOUNCEMENT_SIZE = 93  // No session_id field - matches Apple/Rust implementations
const val TIMESTAMP_WINDOW_SECS = 60L
const val DISCOVERY_INTERVAL_MS = 500L
const val DISCOVERY_TIMEOUT_SECS = 120L

enum class DiscoveryRole(val value: Byte) {
    SENDER(0),
    RECEIVER(1)
}

data class DiscoveryAnnouncement(
    val magic: ByteArray = DISCOVERY_MAGIC.clone(),
    val version: Short = 1,
    var role: DiscoveryRole = DiscoveryRole.SENDER,
    val capabilities: Int = 0,
    var ipAddress: ByteArray = ByteArray(4),
    val port: Short = DISCOVERY_PORT.toShort(),
    var timestamp: Long = System.currentTimeMillis() / 1000,
    var sequence: Int = 0,
    var nonce: ByteArray = ByteArray(32),
    var hmac: ByteArray = ByteArray(32)
) {
    // Note: No session_id - peer matching done via password HMAC + role (matches Apple/Rust)
    fun serialize(): ByteArray {
        val buffer = ByteBuffer.allocate(ANNOUNCEMENT_SIZE)
        buffer.put(magic)
        buffer.putShort(version)
        buffer.put(role.value)
        buffer.putInt(capabilities)
        buffer.put(ipAddress)
        buffer.putShort(port)
        buffer.putLong(timestamp)
        buffer.putInt(sequence)
        buffer.put(nonce)
        buffer.put(hmac)
        return buffer.array()
    }

    fun sign(key: ByteArray) {
        val data = serialize()
        val dataWithoutHmac = data.sliceArray(0 until ANNOUNCEMENT_SIZE - 32)
        hmac = computeHmac(key, dataWithoutHmac)
    }

    fun verify(key: ByteArray): Boolean {
        val data = serialize()
        val dataWithoutHmac = data.sliceArray(0 until ANNOUNCEMENT_SIZE - 32)
        return verifyHmac(key, dataWithoutHmac, hmac)
    }

    fun isTimestampValid(): Boolean {
        val now = System.currentTimeMillis() / 1000
        return kotlin.math.abs(now - timestamp) <= TIMESTAMP_WINDOW_SECS
    }

    companion object {
        fun deserialize(data: ByteArray): DiscoveryAnnouncement? {
            if (data.size < ANNOUNCEMENT_SIZE) return null

            val buffer = ByteBuffer.wrap(data)

            val magic = ByteArray(4)
            buffer.get(magic)
            if (!magic.contentEquals(DISCOVERY_MAGIC)) return null

            val version = buffer.short

            val roleByte = buffer.get()
            val role = when (roleByte) {
                0.toByte() -> DiscoveryRole.SENDER
                1.toByte() -> DiscoveryRole.RECEIVER
                else -> return null
            }

            val capabilities = buffer.int

            val ipAddress = ByteArray(4)
            buffer.get(ipAddress)

            val port = buffer.short
            val timestamp = buffer.long
            val sequence = buffer.int

            val nonce = ByteArray(32)
            buffer.get(nonce)

            val hmac = ByteArray(32)
            buffer.get(hmac)

            return DiscoveryAnnouncement(
                magic, version, role, capabilities,
                ipAddress, port, timestamp, sequence, nonce, hmac
            )
        }

        fun create(
            role: DiscoveryRole,
            localIp: InetAddress,
            sequence: Int
        ): DiscoveryAnnouncement {
            val nonce = ByteArray(32)
            java.security.SecureRandom().nextBytes(nonce)

            return DiscoveryAnnouncement(
                role = role,
                ipAddress = localIp.address,
                timestamp = System.currentTimeMillis() / 1000,
                sequence = sequence,
                nonce = nonce
            )
        }
    }
}

class DiscoveryManager(
    private val context: Context,
    private val key: ByteArray,
    private val role: DiscoveryRole,
    private val localIp: InetAddress,
    private val outputText: (String) -> Unit
) {
    // Note: No session_id - peer matching done via password HMAC + role (matches Apple/Rust)
    private val cancelled = AtomicBoolean(false)
    private var multicastLock: WifiManager.MulticastLock? = null

    fun cancel() {
        cancelled.set(true)
    }

    suspend fun discoverPeer(): Inet4Address? = withContext(Dispatchers.IO) {
        outputText("Starting peer discovery...")

        // Try multicast first
        val multicastResult = try {
            discoverMulticast()
        } catch (e: Exception) {
            outputText("Multicast discovery failed: ${e.message}. Trying unicast fallback...")
            null
        }

        if (multicastResult != null) {
            return@withContext multicastResult
        }

        // Fallback to unicast
        discoverUnicast()
    }

    private suspend fun discoverMulticast(): Inet4Address? = withContext(Dispatchers.IO) {
        // Acquire multicast lock
        val wifiManager = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        multicastLock = wifiManager.createMulticastLock("FlyingCarpetDiscovery")
        multicastLock?.setReferenceCounted(true)
        multicastLock?.acquire()

        try {
            val multicastAddr = InetAddress.getByName(MULTICAST_ADDR)
            val socket = MulticastSocket(DISCOVERY_PORT)
            socket.reuseAddress = true
            socket.soTimeout = 100 // 100ms timeout for non-blocking behavior

            try {
                socket.joinGroup(InetSocketAddress(multicastAddr, DISCOVERY_PORT), NetworkInterface.getByInetAddress(localIp))
            } catch (e: Exception) {
                Log.w("Discovery", "Could not join multicast group: ${e.message}")
            }

            outputText("Listening for peer on multicast $MULTICAST_ADDR:$DISCOVERY_PORT")

            val multicastDest = InetSocketAddress(multicastAddr, DISCOVERY_PORT)
            var sequence = 0
            val startTime = System.currentTimeMillis()

            while (!cancelled.get() && System.currentTimeMillis() - startTime < DISCOVERY_TIMEOUT_SECS * 1000) {
                // Send announcement
                val announcement = DiscoveryAnnouncement.create(
                    role, localIp, sequence
                )
                announcement.sign(key)
                val data = announcement.serialize()

                try {
                    val packet = DatagramPacket(data, data.size, multicastDest)
                    socket.send(packet)
                    sequence++
                } catch (e: Exception) {
                    Log.w("Discovery", "Failed to send multicast: ${e.message}")
                }

                // Check for incoming announcements
                val receiveBuffer = ByteArray(1024)
                val receivePacket = DatagramPacket(receiveBuffer, receiveBuffer.size)

                try {
                    socket.receive(receivePacket)

                    val received = DiscoveryAnnouncement.deserialize(
                        receivePacket.data.sliceArray(0 until receivePacket.length)
                    )

                    if (received != null) {
                        val receivedIp = InetAddress.getByAddress(received.ipAddress)

                        // Skip our own announcements
                        if (receivedIp == localIp) continue

                        // Verify HMAC
                        if (!received.verify(key)) continue

                        // Check timestamp
                        if (!received.isTimestampValid()) continue

                        // Check role (must be opposite)
                        if (received.role == role) continue

                        outputText("Discovered peer at $receivedIp")
                        socket.close()
                        return@withContext receivedIp as? Inet4Address
                    }
                } catch (e: SocketTimeoutException) {
                    // Expected timeout, continue loop
                }

                delay(DISCOVERY_INTERVAL_MS)
            }

            socket.close()
            null
        } finally {
            multicastLock?.release()
            multicastLock = null
        }
    }

    private suspend fun discoverUnicast(): Inet4Address? = withContext(Dispatchers.IO) {
        outputText("Starting unicast subnet scan...")

        val socket = DatagramSocket()
        socket.soTimeout = 100

        // Get subnet from local IP (assume /24)
        val octets = localIp.address

        var sequence = 0
        val startTime = System.currentTimeMillis()

        while (!cancelled.get() && System.currentTimeMillis() - startTime < DISCOVERY_TIMEOUT_SECS * 1000) {
            // Scan the subnet
            for (i in 1..254) {
                val targetIp = InetAddress.getByAddress(
                    byteArrayOf(octets[0], octets[1], octets[2], i.toByte())
                )

                // Skip our own IP
                if (targetIp == localIp) continue

                val announcement = DiscoveryAnnouncement.create(
                    role, localIp, sequence
                )
                announcement.sign(key)
                val data = announcement.serialize()

                try {
                    val packet = DatagramPacket(
                        data, data.size,
                        InetSocketAddress(targetIp, DISCOVERY_PORT)
                    )
                    socket.send(packet)
                } catch (e: Exception) {
                    // Ignore send errors
                }
            }

            sequence++

            // Check for incoming announcements
            val receiveBuffer = ByteArray(1024)
            val receivePacket = DatagramPacket(receiveBuffer, receiveBuffer.size)

            repeat(10) { // Check multiple times before sending again
                try {
                    socket.receive(receivePacket)

                    val received = DiscoveryAnnouncement.deserialize(
                        receivePacket.data.sliceArray(0 until receivePacket.length)
                    )

                    if (received != null) {
                        val receivedIp = InetAddress.getByAddress(received.ipAddress)

                        // Skip our own announcements
                        if (receivedIp == localIp) return@repeat

                        // Verify HMAC
                        if (!received.verify(key)) return@repeat

                        // Check timestamp
                        if (!received.isTimestampValid()) return@repeat

                        // Check role (must be opposite)
                        if (received.role == role) return@repeat

                        outputText("Discovered peer at $receivedIp")
                        socket.close()
                        return@withContext receivedIp as? Inet4Address
                    }
                } catch (e: SocketTimeoutException) {
                    // Expected timeout
                }
            }

            delay(DISCOVERY_INTERVAL_MS)
        }

        socket.close()
        null
    }
}
