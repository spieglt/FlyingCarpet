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
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketException
import java.nio.ByteBuffer
import java.security.MessageDigest

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

const val MAJOR_VERSION: Long = 8
val zero = ByteArray(8) // meant to represent a 64-bit unsigned 0
val one = byteArrayOf(0, 0, 0, 0, 0, 0, 0, 1) // meant to represent a 64-bit unsigned 1
const val chunkSize = 5_000_000
//fun ByteArray.toHex(): String = joinToString(separator = "") { eachByte -> "%02x".format(eachByte) }

class MainViewModel(private val application: Application) : AndroidViewModel(application) {

    lateinit var mode: Mode
    lateinit var peer: Peer
    var peerIP: Inet4Address? = null
    var ssid: String = "" // TODO: reset these to empty string on each run of transfer, or how to not allow reads of getWifiInfo() until localOnlyHotspotCallback.onStarted() has run?
    var password: String = ""
    lateinit var key: ByteArray
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
    lateinit var wifiManager: WifiManager
    lateinit var reservation: WifiManager.LocalOnlyHotspotReservation
    val bluetooth = Bluetooth(application, ::gotPeer, ::gotWifiInfo, ::getWifiInfo) // TODO: better way to do these callbacks?
    lateinit var requestPermissionLauncher: ActivityResultLauncher<String>
    lateinit var barcodeLauncher: ActivityResultLauncher<ScanOptions>
    lateinit var displayQrCode: (String, String) -> Unit
    private val handler = Handler(Looper.getMainLooper())
    private var _output = MutableLiveData<String>()
    val output: LiveData<String>
        get() = _output
    val outputText = { msg: String ->
        GlobalScope.launch(Dispatchers.Main) {
            _output.value = msg
        }
    }
    var qrBitmap: Bitmap? = null

    var _progressBar = MutableLiveData(0)
    val progressBar: LiveData<Int>
        get() = _progressBar

    private var _transferFinished = MutableLiveData(false)
    val transferFinished: LiveData<Boolean>
        get() = _transferFinished
    // this round-trip through postValue is required when screen is rotated during transfer
    // and activity is recreated, so that the new activity's observer catches this LiveData event
    // and calls cleanUpTransfer() on the new activity
    val finishTransfer = { _transferFinished.postValue(true) }

    fun serverIsInitialized() = ::server.isInitialized
    fun clientIsInitialized() = ::client.isInitialized
    fun inputStreamIsInitialized() = ::inputStream.isInitialized
    fun outputStreamIsInitialized() = ::outputStream.isInitialized
    fun reservationIsInitialized() = ::reservation.isInitialized

    fun isHosting(): Boolean {
        return peer == Peer.iOS
                || peer == Peer.macOS
                || (peer == Peer.Android && mode == Mode.Receiving)
    }

    suspend fun startTransfer() {
        outputText("\nStarting Transfer")
        startTCP()
        confirmVersion()
        confirmMode()
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
            // find out how many files we're receiving
            val numFilesBytes = readNBytes(8, inputStream)
            val numFiles = ByteBuffer.wrap(numFilesBytes).long

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
        // cancel transfer
        if (transferCoroutine != null) {
            transferCoroutine!!.cancel()
            transferCoroutine = null
        }
        // close tcp streams
        if (inputStreamIsInitialized()) {
            inputStream.close()
        }
        if (outputStreamIsInitialized()) {
            outputStream.close()
        }
        // close sockets, release port
        if (clientIsInitialized()) {
            client.close()
        }
        if (serverIsInitialized()) {
            server.close()
        }
        // tear down hotspot
        if (reservationIsInitialized()) {
            reservation.close()
        }
    }

    fun connectToPeer() {
        // TODO: set ssid and password to empty string here, to prevent bluetooth from allowing reads of values from previous transfer?
        //    no, because if joining hotspot we may have received this data from QR code scanner already? no, we scan QR code here.
        //    and if we startHotspot, we always want it to be fresh
        //    if we're here and joining and have bluetooth, do we have the wifi info? need to call this only when:
        //        - as central, we've hit the onCharacteristicRead of the gattCallback of bluetoothReceiver
        //        - as peripheral, we've hit onCharacteristicWriteRequest of serverCallback of bluetoothGattServer
        //    both of these will call gotWifiInfo which will call this?
        ssid = ""
        password = ""
        // not using bluetooth, startHotspot or launch barcodeLauncher to joinHotspot
        if (isHosting()) {
            // start hotspot
            startHotspot()
        } else { // joining hotspot
            if (bluetooth.active) {
                joinHotspot()
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

    // hotspot stuff
    private val localOnlyHotspotCallback = object : WifiManager.LocalOnlyHotspotCallback() {
        override fun onFailed(reason: Int) {
            super.onFailed(reason)
            outputText("Hotspot failed: $reason")
        }

        override fun onStarted(res: WifiManager.LocalOnlyHotspotReservation?) {
            super.onStarted(res)

            // check for cancellation
            if (!transferIsRunning) {
                res?.close()
                return
            }

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

            // set key
            val hasher = MessageDigest.getInstance("SHA-256")
            hasher.update(password.encodeToByteArray())
            key = hasher.digest()

            // if not using bluetooth, show the QR code
            if (!bluetooth.active) {
                // android generates ssid and password for us
                displayQrCode(ssid, password)
            }

            outputText("SSID: ${ssid}")
            outputText("Password: ${password}")

            transferCoroutine = GlobalScope.launch {
                try {
                    startTransfer()
                } catch (e: Exception) {
                    outputText("Transfer error: ${e.message}\n")
                }
                finishTransfer()
            }

        }

        override fun onStopped() {
            super.onStopped()
            outputText("Hotspot stopped")
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
//            Log.e("FCLOGS", "Didn't have $requiredPermission")
        } else {
//            Log.i("FCLOGS", "Had $requiredPermission")
            try {
                wifiManager.startLocalOnlyHotspot(localOnlyHotspotCallback, handler)
                outputText("Started hotspot.")
            } catch (e: Exception) {
                e.message?.let { outputText(it) }
                cleanUpTransfer()
            }
        }
    }

    fun joinHotspot() {
        val callback = NetworkCallback()
        outputText("Joining ${ssid}")
        // outputText("Password ${password}")
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

    private fun gotPeer(peerOS: String) {
        peer = when (peerOS) {
            "android" -> Peer.Android
            "ios" -> Peer.iOS
            "linux" -> Peer.Linux
            "macos" -> Peer.macOS
            "windows" -> Peer.Windows
            else -> {
                outputText("Error: peer sent an unsupported OS.")
                return
            }
        }
    }

    private fun gotWifiInfo(ssid: String, password: String, key: ByteArray) {
        this.ssid = ssid
        this.password = password
        this.key = key
        connectToPeer()
    }

    private fun getWifiInfo(): String {
        if (ssid == "" || password == "") {
            return ""
        }
        return "$ssid;$password"
    }

    private suspend fun startTCP() {
        withContext(Dispatchers.IO) {
            if (isHosting()) {
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
            if (isHosting()) {
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
                // peer makes decision
                val isCompatibleBytes = readNBytes(8, inputStream)
                if (ByteBuffer.wrap(isCompatibleBytes).long != 1L) {
                    throw Exception("Peer's version of Flying Carpet is not compatible. Please find links to download the newest version at https://flyingcarpet.spiegl.dev.")
                }
            } else {
                // we make decision
                // for this version, we're only compatible with the same
                if (peerVersion != MAJOR_VERSION) {
                    throw Exception("Peer's version of Flying Carpet is not compatible. Please find links to download the newest version at https://flyingcarpet.spiegl.dev.")
                }
            }
        }
    }

    private suspend fun confirmMode() {
        withContext(Dispatchers.IO) {
            val ourMode = if (mode == Mode.Sending) {
                1L
            } else {
                0L
            }
            if (isHosting()) {
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
                bytesRead += br
            } catch (e: SocketException) {
                throw Exception("Peer connection closed")
            }
        }
        return b
    }

    fun findNewFilename(destinationDir: DocumentFile, filename: String): String {
        var newFileName = filename.split("/").last()
        var fileHandle = destinationDir.findFile(filename)
        var i = 1
        while (fileHandle != null) {
            newFileName = "($i) $filename"
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
}

