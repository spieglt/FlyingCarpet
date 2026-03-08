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
const val ANNOUNCEMENT_SIZE = 93
const val TIMESTAMP_WINDOW_SECS = 60L
const val DISCOVERY_INTERVAL_MS = 500L
const val DISCOVERY_TIMEOUT_SECS = 120L
const val MAX_UNICAST_SCAN_HOSTS = 1024

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

/// Returns the list of host IPs to scan, or null if the subnet is too large.
fun unicastScanTargets(localIp: InetAddress, prefixLength: Int): List<InetAddress>? {
    if (prefixLength > 30 || prefixLength <= 0) return null

    val ipBytes = localIp.address
    val ipInt = ((ipBytes[0].toInt() and 0xFF) shl 24) or
            ((ipBytes[1].toInt() and 0xFF) shl 16) or
            ((ipBytes[2].toInt() and 0xFF) shl 8) or
            (ipBytes[3].toInt() and 0xFF)

    val mask = if (prefixLength == 0) 0 else (-1 shl (32 - prefixLength))
    val network = ipInt and mask
    val broadcast = network or mask.inv()
    val numHosts = broadcast - network - 1

    if (numHosts > MAX_UNICAST_SCAN_HOSTS) return null

    val targets = mutableListOf<InetAddress>()
    for (addr in (network + 1) until broadcast) {
        if (addr == ipInt) continue
        val bytes = byteArrayOf(
            ((addr shr 24) and 0xFF).toByte(),
            ((addr shr 16) and 0xFF).toByte(),
            ((addr shr 8) and 0xFF).toByte(),
            (addr and 0xFF).toByte()
        )
        targets.add(InetAddress.getByAddress(bytes))
    }
    return targets
}

/// Gets the prefix length for the given local IP from NetworkInterface.
fun getPrefixLength(localIp: InetAddress): Int {
    try {
        val networkInterface = NetworkInterface.getByInetAddress(localIp)
        if (networkInterface != null) {
            for (addr in networkInterface.interfaceAddresses) {
                if (addr.address == localIp) {
                    return addr.networkPrefixLength.toInt()
                }
            }
        }
    } catch (e: Exception) {
        Log.w("Discovery", "Could not get prefix length: ${e.message}")
    }
    return 24 // default fallback
}

class DiscoveryManager(
    private val context: Context,
    private val key: ByteArray,
    private val role: DiscoveryRole,
    private val localIp: InetAddress,
    private val outputText: (String) -> Unit
) {
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

        // Fallback to unicast only
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

            val result = CompletableDeferred<Inet4Address?>()

            // Sender coroutine - sends multicast announcements in parallel
            val sender = launch {
                val multicastDest = InetSocketAddress(multicastAddr, DISCOVERY_PORT)
                var sequence = 0
                while (isActive && !cancelled.get()) {
                    val announcement = DiscoveryAnnouncement.create(role, localIp, sequence)
                    announcement.sign(key)
                    val data = announcement.serialize()
                    try {
                        socket.send(DatagramPacket(data, data.size, multicastDest))
                        sequence++
                    } catch (e: Exception) {
                        Log.w("Discovery", "Failed to send multicast: ${e.message}")
                    }
                    delay(DISCOVERY_INTERVAL_MS)
                }
            }

            // Receiver coroutine - listens for announcements in parallel
            val receiver = launch {
                while (isActive && !cancelled.get()) {
                    val receiveBuffer = ByteArray(1024)
                    val receivePacket = DatagramPacket(receiveBuffer, receiveBuffer.size)
                    try {
                        socket.receive(receivePacket)

                        val received = DiscoveryAnnouncement.deserialize(
                            receivePacket.data.sliceArray(0 until receivePacket.length)
                        )

                        if (received != null) {
                            val receivedIp = InetAddress.getByAddress(received.ipAddress)
                            if (receivedIp == localIp) continue
                            if (!received.verify(key)) continue
                            if (!received.isTimestampValid()) continue
                            if (received.role == role) continue

                            outputText("Discovered peer at $receivedIp")
                            result.complete(receivedIp as? Inet4Address)
                            return@launch
                        }
                    } catch (e: SocketTimeoutException) {
                        // Expected timeout, continue loop
                    }
                }
            }

            val peerIp = withTimeoutOrNull(DISCOVERY_TIMEOUT_SECS * 1000) {
                result.await()
            }

            sender.cancel()
            receiver.cancel()
            socket.close()

            peerIp
        } finally {
            multicastLock?.release()
            multicastLock = null
        }
    }

    private suspend fun discoverUnicast(): Inet4Address? = withContext(Dispatchers.IO) {
        val prefixLength = getPrefixLength(localIp)
        val targets = unicastScanTargets(localIp, prefixLength)

        if (targets == null) {
            outputText("Subnet too large for unicast scan (/$prefixLength), relying on multicast only.")
            return@withContext null
        }

        outputText("Starting unicast subnet scan (/$prefixLength, ${targets.size} hosts)...")

        val socket = DatagramSocket()
        socket.soTimeout = 100

        val result = CompletableDeferred<Inet4Address?>()

        // Sender coroutine
        val sender = launch {
            var sequence = 0
            while (isActive && !cancelled.get()) {
                for (targetIp in targets) {
                    if (!isActive || cancelled.get()) return@launch

                    val announcement = DiscoveryAnnouncement.create(role, localIp, sequence)
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
                delay(DISCOVERY_INTERVAL_MS)
            }
        }

        // Receiver coroutine
        val receiver = launch {
            while (isActive && !cancelled.get()) {
                val receiveBuffer = ByteArray(1024)
                val receivePacket = DatagramPacket(receiveBuffer, receiveBuffer.size)
                try {
                    socket.receive(receivePacket)

                    val received = DiscoveryAnnouncement.deserialize(
                        receivePacket.data.sliceArray(0 until receivePacket.length)
                    )

                    if (received != null) {
                        val receivedIp = InetAddress.getByAddress(received.ipAddress)
                        if (receivedIp == localIp) continue
                        if (!received.verify(key)) continue
                        if (!received.isTimestampValid()) continue
                        if (received.role == role) continue

                        outputText("Discovered peer at $receivedIp")
                        result.complete(receivedIp as? Inet4Address)
                        return@launch
                    }
                } catch (e: SocketTimeoutException) {
                    // Expected timeout
                }
            }
        }

        val peerIp = withTimeoutOrNull(DISCOVERY_TIMEOUT_SECS * 1000) {
            result.await()
        }

        sender.cancel()
        receiver.cancel()
        socket.close()

        peerIp
    }
}
