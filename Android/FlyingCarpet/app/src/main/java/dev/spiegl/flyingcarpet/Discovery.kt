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

    // Mirrors the Rust and Apple implementations: a single socket bound to DISCOVERY_PORT
    // receives both multicast and unicast announcements, while a multicast sender and a
    // unicast subnet scan run in parallel (so a network that blocks multicast still works).
    // Announcements are HMAC-signed with the password-derived key; the first valid
    // announcement from the opposite role wins.
    suspend fun discoverPeer(): Inet4Address? = withContext(Dispatchers.IO) {
        outputText("Searching for peer via multicast ($MULTICAST_ADDR) and unicast subnet scan...")

        // Android filters multicast in the WiFi driver unless a MulticastLock is held
        val wifiManager = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        multicastLock = wifiManager.createMulticastLock("FlyingCarpetDiscovery")
        multicastLock?.acquire()

        var recvSocket: MulticastSocket? = null
        var sendSocket: DatagramSocket? = null
        try {
            // Receives multicast and unicast announcements on the discovery port, and
            // sends our multicast announcements.
            val socket = MulticastSocket(DISCOVERY_PORT)
            recvSocket = socket
            socket.soTimeout = 100 // 100ms timeout so the receive loop can check for cancellation
            val multicastAddr = InetAddress.getByName(MULTICAST_ADDR)
            try {
                val networkInterface = NetworkInterface.getByInetAddress(localIp)
                socket.networkInterface = networkInterface
                socket.joinGroup(InetSocketAddress(multicastAddr, DISCOVERY_PORT), networkInterface)
            } catch (e: Exception) {
                Log.w("Discovery", "Could not join multicast group: ${e.message}")
                // unicast discovery still works
            }

            // Sends the unicast subnet scan from an ephemeral port. The peer's
            // announcements always arrive on the discovery port socket above.
            val unicastSocket = DatagramSocket()
            sendSocket = unicastSocket

            coroutineScope {
                val result = CompletableDeferred<Inet4Address?>()

                val multicastSender = launch {
                    val dest = InetSocketAddress(multicastAddr, DISCOVERY_PORT)
                    var sequence = 0
                    while (isActive && !cancelled.get()) {
                        val announcement = DiscoveryAnnouncement.create(role, localIp, sequence)
                        announcement.sign(key)
                        val data = announcement.serialize()
                        try {
                            socket.send(DatagramPacket(data, data.size, dest))
                        } catch (e: Exception) {
                            Log.w("Discovery", "Failed to send multicast: ${e.message}")
                        }
                        sequence++
                        delay(DISCOVERY_INTERVAL_MS)
                    }
                }

                val unicastSender = launch {
                    val prefixLength = getPrefixLength(localIp)
                    val targets = unicastScanTargets(localIp, prefixLength)
                    if (targets == null) {
                        outputText("Subnet too large for unicast scan (/$prefixLength), relying on multicast only.")
                        return@launch
                    }
                    outputText("Scanning ${targets.size} addresses on the local /$prefixLength subnet...")
                    var sequence = 0
                    var loggedSendFailure = false
                    while (isActive && !cancelled.get()) {
                        // sign one announcement per round and reuse its bytes for every host
                        val announcement = DiscoveryAnnouncement.create(role, localIp, sequence)
                        announcement.sign(key)
                        val data = announcement.serialize()
                        for (target in targets) {
                            if (!isActive || cancelled.get()) return@launch
                            try {
                                unicastSocket.send(DatagramPacket(data, data.size, InetSocketAddress(target, DISCOVERY_PORT)))
                            } catch (e: Exception) {
                                // hosts that don't exist are expected; log the first failure
                                // in case the whole scan is broken (e.g. permission denied)
                                if (!loggedSendFailure) {
                                    loggedSendFailure = true
                                    Log.w("Discovery", "Unicast send to $target failed: ${e.message}")
                                }
                            }
                        }
                        sequence++
                        delay(DISCOVERY_INTERVAL_MS)
                    }
                }

                // There's no discovery timeout (the other device may not be started for a
                // long time), so problems are reported inline, once each, while the search
                // continues.
                val receiver = launch {
                    val buffer = ByteArray(1024)
                    val started = System.currentTimeMillis()
                    var receivedPeerPacket = false
                    var warnedQuiet = false
                    var warnedHmac = false
                    var warnedStale = false
                    var warnedRole = false
                    while (isActive && !cancelled.get()) {
                        val packet = DatagramPacket(buffer, buffer.size)
                        try {
                            socket.receive(packet)
                        } catch (e: SocketTimeoutException) {
                            // expected poll timeout: check for cancellation and try again. if
                            // nothing from the peer has arrived after a while, hint at likely
                            // causes (the other device may also simply not be started yet)
                            if (!warnedQuiet && !receivedPeerPacket
                                && System.currentTimeMillis() - started >= 30_000
                            ) {
                                warnedQuiet = true
                                outputText("Still searching. If the other device has already started the transfer, check that both devices are on the same network and that no firewall is blocking UDP port 3290.")
                            }
                            continue
                        }

                        val received = DiscoveryAnnouncement.deserialize(
                            packet.data.sliceArray(0 until packet.length)
                        )
                        if (received == null) {
                            Log.i("Discovery", "Unparseable ${packet.length}-byte datagram from ${packet.address}")
                            continue
                        }

                        val receivedIp = InetAddress.getByAddress(received.ipAddress)
                        if (receivedIp == localIp) continue // our own announcement
                        receivedPeerPacket = true
                        if (!received.verify(key)) {
                            if (!warnedHmac) {
                                warnedHmac = true
                                outputText("Received an announcement that failed authentication. If it came from the other device, check that the password matches on both. Still searching...")
                            }
                            Log.i("Discovery", "Announcement from ${packet.address} failed HMAC check (different password?)")
                            continue
                        }
                        if (!received.isTimestampValid()) {
                            if (!warnedStale) {
                                warnedStale = true
                                outputText("Received an announcement with an out-of-date timestamp. Check that both devices' clocks are set correctly. Still searching...")
                            }
                            Log.i("Discovery", "Announcement from ${packet.address} has a stale timestamp")
                            continue
                        }
                        if (received.role == role) {
                            if (!warnedRole) {
                                warnedRole = true
                                outputText("Received an announcement from a device in the same mode as this one. If it's the other device of this transfer, one side must select Send and the other Receive. Still searching...")
                            }
                            Log.i("Discovery", "Announcement from ${packet.address} has our own role, ignoring")
                            continue
                        }

                        outputText("Discovered peer at ${receivedIp.hostAddress}")
                        result.complete(receivedIp as? Inet4Address)
                        return@launch
                    }
                    result.complete(null)
                }

                // no timeout: wait until the peer is found or the transfer is cancelled
                val peerIp = result.await()
                cancelled.set(true)
                multicastSender.cancel()
                unicastSender.cancel()
                receiver.cancel()
                peerIp
            }
        } finally {
            recvSocket?.close()
            sendSocket?.close()
            multicastLock?.release()
            multicastLock = null
        }
    }
}
