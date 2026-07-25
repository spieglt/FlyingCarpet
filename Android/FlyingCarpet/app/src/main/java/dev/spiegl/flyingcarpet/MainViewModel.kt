package dev.spiegl.flyingcarpet

import android.Manifest
import android.app.Application
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.net.*
import android.net.wifi.WifiManager
import android.net.wifi.WifiNetworkSpecifier
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.activity.result.ActivityResultLauncher
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.documentfile.provider.DocumentFile
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.LiveData
import androidx.lifecycle.MutableLiveData
import com.journeyapps.barcodescanner.ScanOptions
import kotlinx.coroutines.*
import java.io.InputStream
import java.io.OutputStream
import java.net.Inet4Address
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketException
import java.net.SocketTimeoutException
import java.nio.ByteBuffer
import java.security.SecureRandom

const val PORT = 3290

enum class Mode {
    Sending,
    Receiving,
}

enum class Peer {
    Android,
    iOS,
    Linux,
    macOS,
    Windows,
}

enum class ConnectionMode {
    Hotspot,
    SharedNetwork,
}

// v10 is a breaking change: shared network mode and its new protocol are not compatible
// with v9 or earlier. See docs/shared-network-crypto.md in the main repo.
const val MAJOR_VERSION: Long = 10
val zero = ByteArray(8) // meant to represent a 64-bit unsigned 0
val one = byteArrayOf(0, 0, 0, 0, 0, 0, 0, 1) // meant to represent a 64-bit unsigned 1
const val chunkSize = 5_000_000
//fun ByteArray.toHex(): String = joinToString(separator = "") { eachByte -> "%02x".format(eachByte) }

/** One line of transfer log, tagged with a monotonically increasing sequence number. */
data class OutputLine(val seq: Long, val text: String)

class MainViewModel(private val application: Application) : AndroidViewModel(application), BluetoothDelegate {

    lateinit var mode: Mode
    lateinit var peer: Peer
    var peerIP: Inet4Address? = null
    var ssid: String = ""
    var password: String = ""
    // PBKDF2-stretched Noise PSK, derived once per transfer off the main thread (600k
    // iterations); also the source of the discovery HMAC key (deriveDiscoveryKey).
    private lateinit var psk: ByteArray
    var connectionMode: ConnectionMode = ConnectionMode.Hotspot
    var files: MutableList<DocumentFile> = mutableListOf()
    var fileStreams: MutableList<InputStream> = mutableListOf()
    var filePaths: MutableList<String> = mutableListOf() // paths relative to root directory peer is sending to
    lateinit var receiveDir: Uri
    lateinit var sendDir: Uri
    var sendFolder: Boolean = false
    private lateinit var server: ServerSocket // TCP listener, used to release port when transfer fails/ends/is cancelled
    lateinit var client: Socket // TCP socket, used to release port when transfer fails/ends/is cancelled
    lateinit var inputStream: InputStream // incoming TCP stream from peer
    lateinit var outputStream: OutputStream // outgoing TCP stream to peer
    var transferCoroutine: Job? = null
    var transferIsRunning = false
    var hotspotRunning = false
    lateinit var wifiManager: WifiManager
    lateinit var reservation: WifiManager.LocalOnlyHotspotReservation
    lateinit var requestPermissionLauncher: ActivityResultLauncher<String>
    val bluetooth = Bluetooth(application, this)
    lateinit var barcodeLauncher: ActivityResultLauncher<ScanOptions>
    lateinit var displayQrCode: (String, String) -> Unit
    lateinit var cleanUpUi: () -> Unit
    lateinit var enableBluetoothUi: (Boolean) -> Unit
    lateinit var promptForPassword: () -> Unit // shared network mode: sender asks user for the receiver's password
    lateinit var displaySharedNetworkPassword: (String) -> Unit // shared network mode: receiver shows generated password as QR code
    var discoveryManager: DiscoveryManager? = null
    private var discoveryJob: Job? = null // receiver-role background discovery in shared network mode
    private var boundToWifiNetwork = false
    private val handler = Handler(Looper.getMainLooper())
    private var _output = MutableLiveData<OutputLine>()
    val output: LiveData<OutputLine>
        get() = _output

    // The whole transcript is kept here, not in the Activity's saved-state Bundle. Bundles
    // cross Binder, whose transaction buffer is ~1MB for the whole process, so putString()ing
    // a many-file transfer's log risks TransactionTooLargeException on rotation. The ViewModel
    // already outlives configuration changes, so the log rides along whole: no size cap, no
    // serialization, nothing dropped. Both fields are touched only from the main thread, by
    // way of the Dispatchers.Main hop in outputText().
    private val outputLog = StringBuilder()
    private var outputSeq = 0L

    /**
     * The transcript so far, paired with the sequence number of its last line. A recreated
     * Activity seeds its fresh TextView with this, then ignores any [output] line at or below
     * that sequence number — LiveData redelivers its most recent value to a newly registered
     * observer, and that line is already in the seed.
     */
    fun outputSnapshot(): Pair<String, Long> = outputLog.toString() to outputSeq

    override fun outputText(msg: String) {
        // Mirror every user-facing line to logcat under one greppable tag, so a transfer's
        // on-screen log can be pulled off the device with `adb logcat -s FlyingCarpet` instead
        // of being retyped by hand. Logged here, off the main-thread hop below, so the logcat
        // timestamps reflect when each line was produced. Trim only for the log line: the
        // leading blank line some messages carry ("\nStarting Transfer") is for on-screen
        // spacing and would otherwise print as an empty logcat entry.
        Log.i("FlyingCarpet", msg.trim())
        GlobalScope.launch(Dispatchers.Main) {
            outputLog.append(msg).append('\n')
            outputSeq++
            _output.value = OutputLine(outputSeq, msg)
        }
    }

    var qrBitmap: Bitmap? = null

    var progressBarMut = MutableLiveData(0)
    val progressBar: LiveData<Int>
        get() = progressBarMut

    private var _transferFinished = MutableLiveData(false)
    val transferFinished: LiveData<Boolean>
        get() = _transferFinished
    // this round-trip through postValue is required when screen is rotated during transfer
    // and activity is recreated, so that the new activity's observer catches this LiveData event
    // and calls cleanUpTransfer() on the new activity
    val finishTransfer = { _transferFinished.postValue(true) }

    private fun isHosting(): Boolean {
        return peer == Peer.iOS
                || peer == Peer.macOS
                || (peer == Peer.Android && mode == Mode.Receiving)
    }

    // Bluetooth is only used in hotspot mode: in shared network mode the password is
    // exchanged manually (receiver displays it, sender enters or scans it).
    fun usingBluetooth(): Boolean {
        return bluetooth.active && connectionMode == ConnectionMode.Hotspot
    }

    suspend fun startTransfer() {
        outputText("\nStarting Transfer")
        // Derive the hotspot PSK up front, while `password` is still known to be the one the
        // peer joined with. Deriving it at handshake time instead left a multi-second window —
        // startTCP() blocks waiting for the peer to associate and connect — during which a
        // stray callback could clear the credentials out from under the transfer. In shared
        // network mode the PSK is derived even earlier, before discovery, which keys its
        // announcement HMAC from it. Same PSK either way: this is ordering only, not a wire
        // change.
        if (connectionMode == ConnectionMode.Hotspot) {
            withContext(Dispatchers.IO) { psk = derivePsk(password) }
        }
        startTCP()
        // Plaintext preamble on the raw socket: version, then send/receive mode. Every
        // preamble byte, sent and received, is recorded and bound into the Noise prologue
        // below, so tampering with the preamble fails the handshake.
        val recordingIn = RecordingInputStream(inputStream)
        val recordingOut = RecordingOutputStream(outputStream)
        inputStream = recordingIn
        outputStream = recordingOut
        confirmVersion()
        confirmMode()
        inputStream = recordingIn.inner
        outputStream = recordingOut.inner
        // Establish the Noise encrypted transport over the same connection, for both modes,
        // with the preamble transcript bound in as the prologue. The Noise initiator is the
        // TCP client, the responder is the TCP server. Everything after this — file count,
        // metadata, and file data — is confidential and tamper-evident. A wrong password
        // (or a tampered preamble) fails the handshake with a clear message.
        val role = if (connectionMode == ConnectionMode.SharedNetwork) {
            if (mode == Mode.Sending) NoiseRole.INITIATOR else NoiseRole.RESPONDER
        } else {
            if (isHosting()) NoiseRole.RESPONDER else NoiseRole.INITIATOR
        }
        val prologue = if (role == NoiseRole.INITIATOR) {
            buildPrologue(recordingOut.transcript(), recordingIn.transcript())
        } else {
            buildPrologue(recordingIn.transcript(), recordingOut.transcript())
        }
        outputText("Establishing encrypted connection...")
        withContext(Dispatchers.IO) {
            val transport = noiseHandshake(inputStream, outputStream, role, psk, prologue)
            inputStream = transport.input
            outputStream = transport.output
        }
        outputText("Encrypted connection established.")
        // send/receive
        if (mode == Mode.Sending) {
            // tell receiving end how many files we're sending
            val numFilesBytes = longToBigEndianBytes(fileStreams.size.toLong())
            withContext(Dispatchers.IO) {
                outputStream.write(numFilesBytes) // write to receiving end
            }

            // send files
            for (i in 0 until fileStreams.size) {
                outputText("=========================")
                outputText("Sending file ${i + 1} of ${fileStreams.size}. Filename: ${files[i].name}.")
                val path = if (i < filePaths.size) { filePaths[i] } else { "" }
                sendFile(files[i], fileStreams[i], path)
            }

        } else if (mode == Mode.Receiving) {
            // find out how many files we're receiving. sanity bound: no legitimate
            // transfer approaches it, and a corrupt or hostile stream shouldn't be
            // able to put us into a near-endless receive loop.
            val numFilesBytes = readNBytes(8, inputStream)
            val numFiles = ByteBuffer.wrap(numFilesBytes).long
            if (numFiles < 0 || numFiles > 1_000_000) {
                throw Exception("File count $numFiles from peer is out of range")
            }

            // receive files
            for (i in 0 until numFiles) {
                outputText("=========================")
                outputText("Receiving file ${i + 1} of $numFiles")
                receiveFile(i == numFiles - 1)
            }
        }
        outputText("=========================")
        outputText("Transfer complete\n")
    }

    fun cleanUpTransfer() {
        transferIsRunning = false
        // cancel shared network discovery if it's running
        discoveryManager?.cancel()
        discoveryManager = null
        discoveryJob?.cancel()
        discoveryJob = null
        // unbind from the WiFi network if we bound to it for a shared network transfer
        if (boundToWifiNetwork) {
            val connectivityManager = application
                .getSystemService(AppCompatActivity.CONNECTIVITY_SERVICE) as ConnectivityManager
            connectivityManager.bindProcessToNetwork(null)
            boundToWifiNetwork = false
        }
        // cancel transfer
        if (transferCoroutine != null) {
            transferCoroutine!!.cancel()
            transferCoroutine = null
        }
        // close tcp streams
        if (this::inputStream.isInitialized) {
            inputStream.close()
        }
        if (this::outputStream.isInitialized) {
            outputStream.close()
        }
        // close sockets, release port
        if (this::client.isInitialized) {
            client.close()
        }
        if (this::server.isInitialized) {
            server.close()
        }
        // tear down hotspot
        if (this::reservation.isInitialized) {
            reservation.close()
        }
        hotspotRunning = false
        // stop bluetooth functions
        bluetooth.stop(application)
        // clean up UI
        cleanUpUi()
    }

    override fun connectToPeer() {
        // The BLE credential exchange deliberately runs over two GATT connections (see
        // Bluetooth.onConnectionStateChange), so this, its final step, can be reached more
        // than once per transfer. Re-entering after the hotspot is up would clear the
        // ssid/password the peer has already used to join — and startHotspot() below then
        // no-ops because hotspotRunning is set, so nothing puts them back. That left the
        // in-flight transfer deriving its Noise PSK from an empty password, failing the
        // handshake with SecretKeySpec's opaque "Empty key" while the peer, already
        // associated and waiting, saw only a dropped socket. This guard is the single point
        // that enforces "start the hotspot once": the first call always arrives with
        // hotspotRunning false (MainActivity clears it when Start is pressed; only the
        // LocalOnlyHotspot onStarted callback sets it), and every later replay is a no-op.
        if (hotspotRunning) {
            Log.i("Flying Carpet", "connectToPeer() replayed after hotspot start; ignoring")
            return
        }
        ssid = ""
        password = ""
        if (connectionMode == ConnectionMode.SharedNetwork) {
            // no hotspot and no bluetooth: the receiver generates and displays a password,
            // the sender enters or scans it, and discovery finds the peer on the network
            // both devices are already connected to.
            if (mode == Mode.Receiving) {
                password = generatePassword()
                outputText("Password: $password")
                outputText("Enter this password on the sending device, or scan the QR code with it.")
                displaySharedNetworkPassword(password)
                launchSharedNetworkTransfer()
            } else {
                // MainActivity shows a dialog and calls gotSharedNetworkPassword() with the result
                promptForPassword()
            }
            return
        }
        // if we're hosting, startHotspot() will write the wifi details over bluetooth or display the QR code
        // if we're joining and using bluetooth, we read peer's wifi characteristic here, then bluetoothReceiver's gattCallback's onCharacteristicRead will call gotSsid()
        // if we're joining and not using bluetooth, barcodeLauncher will call joinHotspot()
        // but who will call connectToPeer? file/folder pickers in MainActivity if not using bluetooth, or after we write OS if bluetooth
        if (isHosting()) {
            // The BLE exchange has done its job for the host: from here the peer reads/receives
            // the wifi details and joins. Mark it complete so a post-bond GATT reconnection
            // doesn't replay read-OS → write-OS mid-transfer. Scoped to the host path on
            // purpose — the joiner (else branch) is only *starting* its SSID/password reads
            // here, so it still relies on the replay as a retry and must not be gated.
            bluetooth.bluetoothReceiver.exchangeComplete = true
            startHotspot()
        } else { // joining hotspot
            if (bluetooth.active) {
                if (mode == Mode.Sending) {
                    // we're peripheral, and we're joining, and already know peer's OS, so need to
                    // wait for central to write the hotspot details. so nothing to do here.
                } else {
                    // we're central, so read wifi details
                    bluetooth.bluetoothReceiver.read(SSID_CHARACTERISTIC_UUID)
                }
            } else {
                // scan qr code
                val options = ScanOptions()
                options.setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                options.setPrompt("Start transfer on the other device and scan the QR code displayed.")
                options.setOrientationLocked(false)
                barcodeLauncher.launch(options)
            }
        }
    }

    // shared network mode

    // called with the password the user typed or scanned when sending in shared network mode
    fun gotSharedNetworkPassword(entered: String) {
        password = entered
        launchSharedNetworkTransfer()
    }

    private fun launchSharedNetworkTransfer() {
        transferCoroutine = GlobalScope.launch {
            try {
                // Derive the PSK before discovery starts: the discovery announcement HMAC
                // key comes from it. In the coroutine (not the main thread) because PBKDF2
                // at 600k iterations takes a noticeable fraction of a second.
                psk = derivePsk(password)
                findPeerOnSharedNetwork()
                startTransfer()
            } catch (e: CancellationException) {
                // cancelling the transfer (e.g. the Cancel button) is not an error; don't
                // report it. Rethrow so cancellation propagates instead of being swallowed.
                throw e
            } catch (e: Exception) {
                outputText("Transfer error: ${e.message}\n")
            } finally {
                // runs on success, error, and cancellation alike
                finishTransfer()
            }
        }
    }

    private suspend fun findPeerOnSharedNetwork() {
        val connectivityManager = application
            .getSystemService(AppCompatActivity.CONNECTIVITY_SERVICE) as ConnectivityManager
        val (network, localIp) = getSharedNetworkAndIp(connectivityManager)
            ?: throw Exception(
                "No network connection. Shared Network mode requires both devices to be "
                        + "connected to the same network. Connect to WiFi (or wired Ethernet) "
                        + "or use Hotspot mode."
            )
        // route our traffic over this network even if Android prefers another one, e.g.
        // cellular because the local network has no internet access
        connectivityManager.bindProcessToNetwork(network)
        boundToWifiNetwork = true
        outputText("Local IP: ${localIp.hostAddress}")

        // Receiver is TCP server (consistent with hotspot same-platform convention where the
        // receiver hosts). Bind the listener *before* discovery so it's ready when the sender
        // connects immediately after discovering us.
        if (mode == Mode.Receiving) {
            withContext(Dispatchers.IO) {
                server = ServerSocket(PORT)
                server.soTimeout = 1_000 // poll interval so the accept loop in startTCP() notices cancellation
            }
            outputText("TCP listener ready on port $PORT.")
        }

        val role = if (mode == Mode.Sending) DiscoveryRole.SENDER else DiscoveryRole.RECEIVER
        val discovery =
            DiscoveryManager(getApplication(), deriveDiscoveryKey(psk), role, localIp, ::outputText)
        discoveryManager = discovery
        if (mode == Mode.Receiving) {
            // The sender discovers us and connects, and it stops announcing as soon as it
            // hears us — possibly before we ever hear it. So the TCP connection (accepted
            // in startTCP()) is the receiver's completion signal: discovery runs in the
            // background only to announce our presence and surface diagnostics
            // (receiver-role discoverPeer() never returns a peer).
            discoveryJob = GlobalScope.launch(Dispatchers.IO) {
                try {
                    discovery.discoverPeer()
                } catch (e: CancellationException) {
                    // expected: startTransfer() cancels this job once the sender's TCP
                    // connection arrives (and cleanUpTransfer() cancels it on teardown).
                    // Not an error, so don't surface it. Rethrow so cancellation propagates.
                    throw e
                } catch (e: Exception) {
                    outputText("Discovery error: ${e.message}")
                }
            }
        } else {
            // discoverPeer() searches until the peer is found or the transfer is cancelled
            val peer = discovery.discoverPeer() ?: throw Exception("Discovery cancelled.")
            discoveryManager = null
            peerIP = peer
        }
    }

    // Prefer WiFi, but accept Ethernet (e.g. USB-C adapters) so wired devices work too.
    private fun getSharedNetworkAndIp(connectivityManager: ConnectivityManager): Pair<Network, Inet4Address>? {
        var wired: Pair<Network, Inet4Address>? = null
        @Suppress("DEPRECATION")
        for (network in connectivityManager.allNetworks) {
            val capabilities = connectivityManager.getNetworkCapabilities(network) ?: continue
            val isWifi = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
            val isEthernet = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
            if (!isWifi && !isEthernet) continue
            val linkProperties = connectivityManager.getLinkProperties(network) ?: continue
            for (linkAddress in linkProperties.linkAddresses) {
                val address = linkAddress.address
                if (address is Inet4Address && !address.isLoopbackAddress && !address.isLinkLocalAddress) {
                    if (isWifi) return Pair(network, address)
                    if (wired == null) wired = Pair(network, address)
                }
            }
        }
        return wired
    }

    // same charset and length as the desktop version's generate_password():
    // 10 chars ≈ 2^58, so a precomputed PBKDF2 table over the whole password space
    // (possible because the PSK salt is a fixed domain string) is infeasible.
    private fun generatePassword(): String {
        val chars = "23456789abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ"
        val random = SecureRandom()
        return (1..10).map { chars[random.nextInt(chars.length)] }.joinToString("")
    }

    // Retries for up to 30 seconds (matches the desktop and Apple implementations): the
    // receiver may still be finishing discovery when we start connecting.
    private suspend fun connectToSharedNetworkPeer() {
        outputText("Connecting to receiver at ${peerIP?.hostAddress}:$PORT...")
        val deadline = System.currentTimeMillis() + 30_000
        var attempt = 0
        while (true) {
            attempt++
            try {
                val socket = Socket()
                socket.connect(InetSocketAddress(peerIP, PORT), 5000)
                client = socket
                return
            } catch (e: Exception) {
                if (System.currentTimeMillis() >= deadline) {
                    throw Exception("Failed to connect to peer after $attempt attempts: ${e.message}")
                }
                outputText("Connection attempt $attempt failed, retrying...")
                delay(2000)
            }
        }
    }

    // hotspot stuff
    private val localOnlyHotspotCallback = object : WifiManager.LocalOnlyHotspotCallback() {
        override fun onFailed(reason: Int) {
            super.onFailed(reason)
            outputText("Hotspot failed: $reason")
            hotspotRunning = false
        }

        override fun onStarted(res: WifiManager.LocalOnlyHotspotReservation?) {
            super.onStarted(res)

            // check for cancellation. if the transfer finished or was cancelled before this
            // callback arrived, tear the hotspot back down and leave hotspotRunning false so the
            // next transfer can start one. this must come before setting hotspotRunning: a stray
            // start (e.g. a leftover BT connection re-driving connectToPeer after a completed
            // transfer) otherwise stuck the flag true and made the next real startHotspot() log
            // "hotspot already running" and hang.
            if (!transferIsRunning) {
                res?.close()
                return
            }

            // set flag so we know not to start this twice
            hotspotRunning = true

            if (res != null) {
                reservation = res
            } else {
                outputText("Failed to get hotspot reservation")
                cleanUpTransfer()
                return
            }

            // get ssid and password
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
                val info = reservation.wifiConfiguration
                info?.let {
                    ssid = it.SSID
                    password = it.preSharedKey
                }
            } else {
                val info = reservation.softApConfiguration
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    info.wifiSsid?.let { ssid = it.toString() }
                } else {
                    info.ssid?.let { ssid = it }
                }
                info.passphrase?.let { password = it }
            }

            // ensure no quotes around the ssid, not sure why this is necessary
            ssid = ssid.replace("\"", "")

            if (bluetooth.active) {
                if (mode == Mode.Sending) {
                    // we're peripheral, and hosting, so just need to wait for the central to read from our
                    // wifi characteristic. nothing to do here.
                } else {
                    // write the wifi details to peer
                    bluetooth.bluetoothReceiver.write(SSID_CHARACTERISTIC_UUID, ssid.toByteArray())
                }
            } else {
                // android generates ssid and password for us
                displayQrCode(ssid, password)
            }

            outputText("SSID: $ssid")
            outputText("Password: $password")

            transferCoroutine = GlobalScope.launch {
                try {
                    startTransfer()
                } catch (e: CancellationException) {
                    // cancelling the transfer is not an error; rethrow so cancellation
                    // propagates instead of being swallowed and reported.
                    throw e
                } catch (e: Exception) {
                    outputText("Transfer error: ${e.message}\n")
                } finally {
                    // runs on success, error, and cancellation alike
                    finishTransfer()
                }
            }

        }

        override fun onStopped() {
            super.onStopped()
            outputText("Hotspot stopped")
            hotspotRunning = false
        }
    }

    fun startHotspot() {
        val requiredPermission = if (Build.VERSION.SDK_INT < 33) {
            Manifest.permission.ACCESS_FINE_LOCATION
        } else {
            Manifest.permission.NEARBY_WIFI_DEVICES
        }
        if (ActivityCompat.checkSelfPermission(
                application, requiredPermission
            ) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissionLauncher.launch(requiredPermission)
        } else {
            try {
                if (!hotspotRunning) {
                    wifiManager.startLocalOnlyHotspot(localOnlyHotspotCallback, handler)
                    outputText("Started hotspot.")
                } else {
                    Log.e("Flying Carpet", "startHotspot() called when hotspot already running")
                }
            } catch (e: Exception) {
                e.message?.let { outputText(it) }
                cleanUpTransfer()
            }
        }
    }

    fun joinHotspot() {
        val callback = NetworkCallback()
        outputText("Joining $ssid")
        val specifier = WifiNetworkSpecifier.Builder()
            .setSsid(ssid)
            .setWpa2Passphrase(password)
            .build()
        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .setNetworkSpecifier(specifier)
            .build()
        val connectivityManager =
            application.getSystemService(AppCompatActivity.CONNECTIVITY_SERVICE) as ConnectivityManager
        callback.connectivityManager = connectivityManager
        peerIP = null // we check this in NetworkCallback so that we only start the transfer once per joinHotspot invocation
        connectivityManager.requestNetwork(request, callback)
    }

    override fun gotPeer(peerOS: String) {
        peer = when (peerOS) {
            "android" -> Peer.Android
            "ios" -> Peer.iOS
            "linux" -> Peer.Linux
            "mac" -> Peer.macOS
            "windows" -> Peer.Windows
            else -> {
                outputText("Error: peer sent an unsupported OS.")
                return
            }
        }
        if (mode == Mode.Sending) {
            connectToPeer()
        } else {
            bluetooth.bluetoothReceiver.write(OS_CHARACTERISTIC_UUID, "android".toByteArray())
        }
    }

    override fun gotSsid(ssid: String) {
        this.ssid = if (ssid == NO_SSID) { "" } else ssid
    }

    override fun gotPassword(password: String) {
        this.password = password
        if (this.ssid == "") {
            val (ssid, _) = getSsidAndKey(password)
            this.ssid = ssid
        }
        // The joiner's last BLE step, and where its half of the exchange is complete — the
        // counterpart to connectToPeer() setting this on the host path. Without it, a joining
        // device never set the flag at all and so ran the whole transfer with every
        // exchangeComplete guard disarmed: the peer's deliberate teardown (Linux removes its
        // GATT service a second after we read the password, then hangs up) arrived looking like
        // a live failure, and the service-changed rediscovery in Bluetooth.onServiceChanged
        // re-ran into a database with no Flying Carpet service in it and failed the transfer.
        // Both joiner roles land here — central by reading this characteristic, peripheral by
        // having it written to us — so this one assignment covers both. Skip an empty password:
        // that means the peer's hotspot isn't up yet, the exchange is *not* done, and the
        // replay this flag suppresses is the retry we need.
        if (password != "") {
            bluetooth.bluetoothReceiver.exchangeComplete = true
        }
        joinHotspot()
    }

    override fun getWifiInfo(): Pair<String, String> {
        // TODO: put mutex around this? and when setting it?
        Log.i("Bluetooth", "In getWifiInfo")
        if (ssid == "" || password == "") {
            return Pair("", "")
        }
        return Pair(ssid, password)
    }

    private suspend fun startTCP() {
        withContext(Dispatchers.IO) {
            if (connectionMode == ConnectionMode.SharedNetwork) {
                // receiver is TCP server, sender connects. the server socket was bound
                // before discovery started, in findPeerOnSharedNetwork().
                if (mode == Mode.Receiving) {
                    outputText("Waiting for TCP connection from sender...")
                    // No timeout: the sender may not be started for a long time. Keep
                    // listening until it connects or the transfer is cancelled (the 1s
                    // soTimeout set at bind is just a poll so cancellation is noticed;
                    // cleanUpTransfer() also closes the server socket).
                    while (true) {
                        try {
                            client = server.accept()
                            break
                        } catch (e: SocketTimeoutException) {
                            if (!isActive) throw CancellationException("Transfer cancelled.")
                        }
                    }
                    // the sender is connected: stop announcing
                    discoveryManager?.cancel()
                    discoveryManager = null
                    discoveryJob?.cancel()
                    discoveryJob = null
                    peerIP = client.inetAddress as? Inet4Address
                    outputText("TCP connection accepted")
                } else {
                    connectToSharedNetworkPeer()
                    outputText("TCP connection established")
                }
            } else if (isHosting()) {
                server = ServerSocket(PORT)
                client = server.accept()
            } else {
                client = Socket(peerIP, 3290)
            }
            client.sendBufferSize = chunkSize * 2
            client.receiveBufferSize = chunkSize * 2
            inputStream = client.getInputStream()
            outputStream = client.getOutputStream()
        }
    }

    private suspend fun confirmVersion() {
        withContext(Dispatchers.IO) {
            val peerVersion: Long
            if (connectionMode == ConnectionMode.SharedNetwork) {
                // symmetric: both sides send their version, then read the peer's.
                // safe from deadlock because TCP buffers the 8-byte writes.
                outputStream.write(longToBigEndianBytes(MAJOR_VERSION))
                peerVersion = ByteBuffer.wrap(readNBytes(8, inputStream)).long
            } else if (isHosting()) {
                // wait for peer's version
                val peerVersionBytes = readNBytes(8, inputStream)
                peerVersion = ByteBuffer.wrap(peerVersionBytes).long
                // send our version
                outputStream.write(longToBigEndianBytes(MAJOR_VERSION))
            } else {
                // send our version
                outputStream.write(longToBigEndianBytes(MAJOR_VERSION))
                // wait for peer's version
                val peerVersionBytes = readNBytes(8, inputStream)
                peerVersion = ByteBuffer.wrap(peerVersionBytes).long
            }
            if (peerVersion < MAJOR_VERSION) {
                // peer's version is lower, so we make the decision and report it to them.
                // v10 is a clean break from earlier versions; if transferring with a higher
                // version, that version decides compatibility.
                if (peerVersion >= 10) {
                    outputStream.write(one)
                } else {
                    outputStream.write(zero)
                    throw Exception("The other device is running Flying Carpet version $peerVersion, which is not compatible with this version ($MAJOR_VERSION). Please update both devices to the latest version at https://flyingcarpet.spiegl.dev.")
                }
            } else if (peerVersion > MAJOR_VERSION) {
                // peer's version is higher, so they make the decision
                val isCompatibleBytes = readNBytes(8, inputStream)
                if (ByteBuffer.wrap(isCompatibleBytes).long != 1L) {
                    throw Exception("The other device is running Flying Carpet version $peerVersion, which is not compatible with this version ($MAJOR_VERSION). Please update both devices to the latest version at https://flyingcarpet.spiegl.dev.")
                }
            } // otherwise versions match, implicitly compatible
        }
    }

    private suspend fun confirmMode() {
        withContext(Dispatchers.IO) {
            val ourMode = if (mode == Mode.Sending) {
                1L
            } else {
                0L
            }
            if (connectionMode == ConnectionMode.SharedNetwork) {
                // symmetric: both sides send their mode, read the peer's, and verify they're opposite
                outputStream.write(if (ourMode == 1L) one else zero)
                val peerMode = ByteBuffer.wrap(readNBytes(8, inputStream)).long
                if (peerMode == ourMode) {
                    throw Exception("Both ends of the transfer selected $mode")
                }
            } else if (isHosting()) {
                // we're hosting, so wait for guest to say what mode they selected, compare to our own, and report back
                val peerModeBytes = readNBytes(8, inputStream)
                val peerMode = ByteBuffer.wrap(peerModeBytes).long
                if (ourMode == peerMode) {
                    outputStream.write(zero)
                    throw Exception("Both ends of the transfer selected $mode")
                } else {
                    // write success to guest
                    outputStream.write(one)
                }
            } else {
                // we're joining, so tell host what mode we selected and wait for confirmation that they don't match
                // if we're in this branch, we're not hosting, so we will have joined a hotspot, so onLinkPropertiesChanged() will have
                // been called, so peerIP should not be null
                if (mode == Mode.Sending) {
                    outputStream.write(one)
                } else {
                    outputStream.write(zero)
                }
                // wait to ensure host responds that mode selection was correct
                val confirmationBytes = readNBytes(8, inputStream)
                val confirmation = ByteBuffer.wrap(confirmationBytes).long
                if (confirmation == 0L) {
                    throw Exception("Both ends of the transfer selected $mode")
                }
            }
        }
    }

    fun readNBytes(n: Int, inputStream: InputStream): ByteArray {
        val b = ByteArray(n)
        var bytesRead = 0
        while (bytesRead < n) {
            try {
                val br = inputStream.read(b, bytesRead, n - bytesRead)
                if (br == -1) {
                    throw Exception("Peer connection closed")
                }
                bytesRead += br
            } catch (e: SocketException) {
                throw Exception("Peer connection closed")
            }
        }
        return b
    }

    fun findNewFilename(destinationDir: DocumentFile, filename: String): String {
        // work with the base name: destinationDir is already the file's parent
        // directory, and a "(n) name" alternative must not contain separators
        val base = filename.split("/").last()
        var newFileName = base
        var fileHandle = destinationDir.findFile(newFileName)
        var i = 1
        while (fileHandle != null) {
            newFileName = "($i) $base"
            fileHandle = destinationDir.findFile(newFileName)
            i++
        }
        return newFileName
    }

    fun getOutputStreamForFile(destinationDir: DocumentFile, filename: String): OutputStream {
        val newFile =
            destinationDir.createFile("*/*", filename) ?: throw Exception("Could not create file URI")
        return getApplication<Application>().contentResolver.openOutputStream(newFile.uri)
            ?: throw Exception("Could not open output stream to new file")
    }

    // used when we join a hotspot
    inner class NetworkCallback : ConnectivityManager.NetworkCallback() {
        lateinit var connectivityManager: ConnectivityManager
        override fun onAvailable(network: Network) {
            super.onAvailable(network)
            connectivityManager.bindProcessToNetwork(network)
        }

        override fun onLost(network: Network) {
            super.onLost(network)
            connectivityManager.bindProcessToNetwork(null)
            outputText("Disconnected from hotspot")
            _transferFinished.postValue(true)
        }

        override fun onUnavailable() {
            super.onUnavailable()
            connectivityManager.bindProcessToNetwork(null)
            outputText("Failed to connect to hotspot")
            _transferFinished.postValue(true)
        }

        // this is our findGateway(), so after we get the gateway/dhcp server ip we're ready to confirm mode and launch transfer
        override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) {
            super.onLinkPropertiesChanged(network, linkProperties)
            // check if transfer was cancelled before this callback ran
            if (!transferIsRunning) {
                return
            }
            // this was set to null in joinHotspot right before requesting the network that triggers this function.
            // check that it's null so we only start the transfer once per joinHotspot invocation
            if (peerIP == null) {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    linkProperties.dhcpServerAddress?.let { peerIP = it }
                } else {
                    for (route in linkProperties.routes) {
                        if (route.isDefaultRoute) {
                            peerIP = route.gateway as Inet4Address?
                        }
                    }
                }
                transferCoroutine = GlobalScope.launch {
                    try {
                        startTransfer()
                    } catch (e: Exception) {
                        outputText("Transfer error: ${e.message}\n")
                    }
                    _transferFinished.postValue(true)
                }
            }
        }
//
//        override fun onBlockedStatusChanged(network: Network, blocked: Boolean) {
//            super.onBlockedStatusChanged(network, blocked)
//            outputText("blocked status changed")
//        }
//
//        override fun onCapabilitiesChanged(
//            network: Network,
//            networkCapabilities: NetworkCapabilities
//        ) {
//            super.onCapabilitiesChanged(network, networkCapabilities)
//            outputText("capabilities changed")
//        }
//
//        override fun onLosing(network: Network, maxMsToLive: Int) {
//            super.onLosing(network, maxMsToLive)
//            outputText("losing")
//        }
    }
    override fun bluetoothFailed() {
        // Once the credential exchange is done, BLE has nothing left to contribute to this
        // transfer — but the peer's BLE teardown is still to come, and from here it is
        // indistinguishable from a failure. Linux removes its GATT service a second after we
        // read the password and then disconnects the link (core/src/linux/bluetooth.rs), which
        // lands as a Service Changed indication, then a disconnect, plus whatever any read or
        // write already in flight returns. Each of those reaches a different one of the ~ten
        // bluetoothFailed() call sites, so gate the teardown itself rather than every caller:
        // after the exchange a BLE failure is a log line, not a reason to kill a transfer that
        // is running over Wi-Fi. Observed 2026-07-25 aborting a Linux->Android transfer between
        // "Joining flyingCarpet_79e9" and the hotspot association.
        if (bluetooth.bluetoothReceiver.exchangeComplete) {
            Log.i("Flying Carpet", "Bluetooth failed after the credential exchange; not failing the transfer")
            return
        }
        enableBluetoothUi(false)
        cleanUpTransfer()
    }
}
