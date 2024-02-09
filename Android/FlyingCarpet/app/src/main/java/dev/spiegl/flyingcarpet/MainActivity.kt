package dev.spiegl.flyingcarpet

import android.Manifest
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.NetworkCapabilities.NET_CAPABILITY_INTERNET
import android.net.NetworkCapabilities.TRANSPORT_WIFI
import android.net.NetworkRequest
import android.net.Uri
import android.net.wifi.WifiManager
import android.net.wifi.WifiNetworkSpecifier
import android.os.Build
import android.os.Bundle
import android.view.View
import android.widget.Button
import android.widget.CheckBox
import android.widget.ImageView
import android.widget.ProgressBar
import android.widget.TextView
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.content.res.AppCompatResources
import androidx.core.app.ActivityCompat
import androidx.core.view.isInvisible
import androidx.core.view.isVisible
import androidx.documentfile.provider.DocumentFile
import androidx.lifecycle.ViewModelProvider
import com.google.android.material.button.MaterialButtonToggleGroup
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import dev.spiegl.flyingcarpet.R.id
import dev.spiegl.flyingcarpet.R.layout
import kotlinx.coroutines.GlobalScope
import kotlinx.coroutines.launch
import java.security.MessageDigest

class MainActivity : AppCompatActivity() {
    lateinit var viewModel: MainViewModel
    private lateinit var outputBox: TextView
    private lateinit var progressBar: ProgressBar
    private lateinit var requestPermissionLauncher: ActivityResultLauncher<String>
    private lateinit var filePicker: ActivityResultLauncher<Array<String>>
    private lateinit var folderPicker: ActivityResultLauncher<Uri?>
    private lateinit var barcodeLauncher: ActivityResultLauncher<ScanOptions>

    // hotspot stuff
    private val localOnlyHotspotCallback = object : WifiManager.LocalOnlyHotspotCallback() {
        override fun onFailed(reason: Int) {
            super.onFailed(reason)
            viewModel.outputText("Hotspot failed: $reason")
        }

        override fun onStarted(res: WifiManager.LocalOnlyHotspotReservation?) {
            super.onStarted(res)

            // check for cancellation
            if (!viewModel.transferIsRunning) {
                res?.close()
                return
            }

            if (res != null) {
                viewModel.reservation = res
            } else {
                viewModel.outputText("Failed to get hotspot reservation")
                cleanUpTransfer()
                return
            }

            val info = viewModel.reservation.softApConfiguration
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                info.wifiSsid?.let { viewModel.ssid = it.toString() }
            } else {
                info.ssid?.let { viewModel.ssid = it }
            }
            info.passphrase?.let { viewModel.password = it }

            // ensure no quotes around the ssid, not sure why this is necessary
            viewModel.ssid = viewModel.ssid.replace("\"", "")

            // set key
            val hasher = MessageDigest.getInstance("SHA-256")
            hasher.update(viewModel.password.encodeToByteArray())
            viewModel.key = hasher.digest()

            // android generates ssid and password for us
            if (viewModel.peer == Peer.iOS || viewModel.peer == Peer.Android) {
                // display qr code
                val qrCode = findViewById<ImageView>(id.qrCodeView)
                viewModel.qrBitmap = getQrCodeBitmap(viewModel.ssid, viewModel.password)
                qrCode.setImageBitmap(viewModel.qrBitmap)
            } else { // peer is macOS, because if windows or linux we wouldn't be hosting
                val alertFragment = Alert(viewModel.ssid, viewModel.password)
                alertFragment.show(supportFragmentManager, "alert")
            }

            viewModel.outputText("SSID: ${viewModel.ssid}")
            viewModel.outputText("Password: ${viewModel.password}")

            viewModel.transferCoroutine = GlobalScope.launch {
                try {
                    viewModel.startTransfer()
                } catch (e: Exception) {
                    viewModel.outputText("Transfer error: ${e.message}\n")
                }
                viewModel.finishTransfer()
            }

        }

        override fun onStopped() {
            super.onStopped()
            viewModel.outputText("Hotspot stopped")
        }
    }

    private fun startHotspot() {
        val requiredPermission = if (Build.VERSION.SDK_INT < 33) {
            Manifest.permission.ACCESS_FINE_LOCATION
        } else {
            Manifest.permission.NEARBY_WIFI_DEVICES
        }
        if (ActivityCompat.checkSelfPermission(
                applicationContext, requiredPermission
            ) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissionLauncher.launch(requiredPermission)
//            Log.e("FCLOGS", "Didn't have $requiredPermission")
        } else {
//            Log.i("FCLOGS", "Had $requiredPermission")
            try {
                viewModel.wifiManager.startLocalOnlyHotspot(localOnlyHotspotCallback, viewModel.handler)
                viewModel.outputText("Started hotspot.")
            } catch (e: Exception) {
                e.message?.let { viewModel.outputText(it) }
                cleanUpTransfer()
            }
        }
    }

    private fun joinHotspot() {
        val callback = viewModel.NetworkCallback()
        viewModel.outputText("Joining ${viewModel.ssid}")
        // outputText("Password ${viewModel.password}")
        val specifier = WifiNetworkSpecifier.Builder()
            .setSsid(viewModel.ssid)
            .setWpa2Passphrase(viewModel.password)
            .build()
        val request = NetworkRequest.Builder()
            .addTransportType(TRANSPORT_WIFI)
            .removeCapability(NET_CAPABILITY_INTERNET)
            .setNetworkSpecifier(specifier)
            .build()
        val connectivityManager =
            applicationContext.getSystemService(CONNECTIVITY_SERVICE) as ConnectivityManager
        callback.connectivityManager = connectivityManager
        viewModel.peerIP = null // we check this in NetworkCallback so that we only start the transfer once per joinHotspot invocation
        connectivityManager.requestNetwork(request, callback)
    }

    private fun connectToPeer() {
        // if windows/linux or android sending, join hotspot. if ios/mac or android receiving, start hotspot.
        if (viewModel.isHosting()) {
            // start hotspot
            startHotspot()
        } else { // joining hotspot
            // scan qr code
            val options = ScanOptions()
            options.setDesiredBarcodeFormats(ScanOptions.QR_CODE)
            options.setPrompt("Start transfer on the other device and scan the QR code displayed.")
            options.setOrientationLocked(false)
            barcodeLauncher.launch(options)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(layout.activity_main)

//        viewModel = ViewModelProvider(this)[MainViewModel::class.java]
        viewModel = ViewModelProvider(this).get(MainViewModel::class.java)

        // set up file and folder pickers
        filePicker =
            registerForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
                viewModel.files = mutableListOf()
                viewModel.fileStreams = mutableListOf()
                if (uris.isEmpty()) {
                    viewModel.outputText("No files selected.")
                    cleanUpTransfer()
                    return@registerForActivityResult
                }
                for (uri in uris) {
                    val file = DocumentFile.fromSingleUri(applicationContext, uri)
                    if (file != null) {
                        viewModel.files.add(file)
                    } else {
                        viewModel.outputText("Could not open file")
                        cleanUpTransfer()
                        return@registerForActivityResult
                    }
                    val stream = applicationContext.contentResolver.openInputStream(uri)
                    if (stream != null) {
                        viewModel.fileStreams.add(stream)
                    } else {
                        viewModel.outputText("Could not open file stream")
                        cleanUpTransfer()
                        return@registerForActivityResult
                    }
                }
                connectToPeer()
            }
        folderPicker =
            registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
                uri?.let {
                    if (viewModel.mode == Mode.Sending) {
                        viewModel.files = mutableListOf()
                        viewModel.fileStreams = mutableListOf()
                        viewModel.filePaths = mutableListOf()
                        val dir = DocumentFile.fromTreeUri(getApplication(), it) ?: run {
                            viewModel.outputText("Could not get DocumentFile from selected directory.")
                            cleanUpTransfer()
                            return@registerForActivityResult
                        }
                        val filesAndPaths = getFilesInDir(dir, "")
                        for (fileAndPath in filesAndPaths) {
                            val file = fileAndPath.first
                            val path = fileAndPath.second
                            viewModel.files.add(file)
                            viewModel.filePaths.add(path)
                            val stream = applicationContext.contentResolver.openInputStream(file.uri)
                            if (stream != null) {
                                viewModel.fileStreams.add(stream)
                            } else {
                                viewModel.outputText("Could not open file stream")
                                cleanUpTransfer()
                                return@registerForActivityResult
                            }
                        }
                        viewModel.sendDir = it
                    } else {
                        viewModel.receiveDir = it
                    }
                    connectToPeer()
                } ?: run {
                    viewModel.outputText("No folder selected.")
                    cleanUpTransfer()
                    return@registerForActivityResult
                }
            }

        // set up permissions request
        viewModel.wifiManager = applicationContext.getSystemService(WIFI_SERVICE) as WifiManager
        requestPermissionLauncher =
            registerForActivityResult(ActivityResultContracts.RequestPermission()) { isGranted: Boolean ->
                if (isGranted) {
                    // Permission is granted. Continue the action or workflow in your app.
                    viewModel.outputText("Permission granted.")
                    // start hotspot here
                    startHotspot()
                } else {
                    // Explain to the user that the feature is unavailable because the
                    // features requires a permission that the user has denied. At the
                    // same time, respect the user's decision. Don't link to system
                    // settings in an effort to convince the user to change their
                    // decision.
                    val permission = if (Build.VERSION.SDK_INT < 33) {
                        "fine location"
                    } else {
                        "nearby device"
                    }
                    viewModel.outputText(
                        "The Android WifiManager requires $permission permission to start hotspot. "
                                + "This data is not collected. "
                                + "Start transfer again if you would like to grant permission."
                    )
                    cleanUpTransfer()
                }
            }


        barcodeLauncher = registerForActivityResult(ScanContract()) { result ->
            if (result.contents == null) {
                viewModel.outputText("Scan cancelled, exiting transfer.")
                cleanUpTransfer()
            } else {
                val ssidAndPassword = result.contents.split(';')
                if (ssidAndPassword.count() > 1) {
                    viewModel.ssid = ssidAndPassword[0]
                    viewModel.password = ssidAndPassword[1]
                    // make sha256 hash of password
                    val hasher = MessageDigest.getInstance("SHA-256")
                    hasher.update(viewModel.password.encodeToByteArray())
                    viewModel.key = hasher.digest()
                } else {
                    viewModel.password = ssidAndPassword[0]
                    // make sha256 hash of password
                    val hasher = MessageDigest.getInstance("SHA-256")
                    hasher.update(viewModel.password.encodeToByteArray())
                    viewModel.key = hasher.digest()
                    viewModel.ssid =
                        "flyingCarpet_%02x%02x".format(viewModel.key[0], viewModel.key[1])
                }
                // join hotspot
                joinHotspot()
            }
        }

        outputBox = findViewById(id.outputBox)
        viewModel.output.observe(this) { msg ->
            outputBox.append(msg + '\n')
        }
        progressBar = findViewById(id.progressBar)
        viewModel.progressBar.observe(this) { value ->
            progressBar.progress = value
        }
        viewModel.transferFinished.observe(this) { finished ->
            // this was firing because when we started observing, we were running cleanUpTransfer()
            // no matter what. and then _transferFinished was true. now initializing as false.
            if (finished) {
                cleanUpTransfer()
            }
        }

        // start button
        val startButton = findViewById<Button>(id.startButton)
        startButton.setOnClickListener {

            // determine send/receive, peer, show file pickers, show or read qr code, or display wifi info
            // then start or join tcp server and start sending or receiving files

            // register that the transfer is running. this is needed so that if the hotspot is kicked off, then the cancel button is hit,
            // the hotspot onStarted callback can bail out.
            viewModel.transferIsRunning = true

            // disable UI elements while transfer is running
            toggleUI(false)

            // prevent screen rotation while transfer is running
            requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_LOCKED

            // get mode
            val modeGroup = findViewById<MaterialButtonToggleGroup>(id.modeGroup)
            val selectedMode = modeGroup.checkedButtonId
            this.viewModel.mode = when (selectedMode) {
                id.sendButton -> Mode.Sending
                id.receiveButton -> Mode.Receiving
                else -> {
                    viewModel.outputText("Must select whether this device is sending or receiving.")
                    cleanUpTransfer()
                    return@setOnClickListener
                }
            }

            // get peer
            val peerGroup = findViewById<MaterialButtonToggleGroup>(id.peerGroup)
            val selectedPeer = peerGroup.checkedButtonId
            this.viewModel.peer = when (selectedPeer) {
                id.androidButton -> Peer.Android
                id.iosButton -> Peer.iOS
                id.linuxButton -> Peer.Linux
                id.macButton -> Peer.macOS
                id.windowsButton -> Peer.Windows
                else -> {
                    viewModel.outputText("Must select operating system of other device.")
                    cleanUpTransfer()
                    return@setOnClickListener
                }
            }

            // get whether we're sending a folder
            val sendFolderCheckBox = findViewById<CheckBox>(id.sendFolderCheckBox)
            this.viewModel.sendFolder = sendFolderCheckBox.isChecked

            when (viewModel.mode) {
                Mode.Sending -> {
                    if (viewModel.sendFolder) {
                        folderPicker.launch(Uri.EMPTY)
                    } else {
                        filePicker.launch(arrayOf("*/*"))
                    }
                }
                Mode.Receiving -> folderPicker.launch(Uri.EMPTY)
            }

        }

        // cancel button
        val cancelButton = findViewById<Button>(id.cancelButton)
        cancelButton.setOnClickListener {
            cleanUpTransfer()
        }

        // sending folder checkbox
        val sendFolderCheckBox = findViewById<CheckBox>(id.sendFolderCheckBox)

        // send button
        val sendButton = findViewById<Button>(id.sendButton)
        sendButton.setOnClickListener {
            startButton.text = getString(R.string.selectFiles)
            sendFolderCheckBox.visibility = View.VISIBLE
        }

        // receive button
        val receiveButton = findViewById<Button>(id.receiveButton)
        receiveButton.setOnClickListener {
            startButton.text = getString(R.string.selectFolder)
            sendFolderCheckBox.visibility = View.GONE
        }

        // about button
        val aboutButton = findViewById<TextView>(id.aboutButton)
        aboutButton.setOnClickListener {
            val aboutFragment = About()
            aboutFragment.show(supportFragmentManager, "alert")
        }

    }

    fun cleanUpTransfer() {
        viewModel.transferIsRunning = false
        // cancel transfer
        if (viewModel.transferCoroutine != null) {
            viewModel.transferCoroutine!!.cancel()
            viewModel.transferCoroutine = null
        }
        // close tcp streams
        if (viewModel.inputStreamIsInitialized()) {
            viewModel.inputStream.close()
        }
        if (viewModel.outputStreamIsInitialized()) {
            viewModel.outputStream.close()
        }
        // close sockets, release port
        if (viewModel.clientIsInitialized()) {
            viewModel.client.close()
        }
        if (viewModel.serverIsInitialized()) {
            viewModel.server.close()
        }
        // tear down hotspot
        if (viewModel.reservationIsInitialized()) {
            viewModel.reservation.close()
        }
        // toggle UI and replace icon
        runOnUiThread {
            requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
            toggleUI(true)
            val qrCode = findViewById<ImageView>(id.qrCodeView)
            val drawable = AppCompatResources.getDrawable(applicationContext, R.drawable.icon1024)
            qrCode.setImageDrawable(drawable)
        }
    }

    private fun toggleUI(enabled: Boolean) {
        findViewById<Button>(id.sendButton).isEnabled = enabled
        findViewById<Button>(id.receiveButton).isEnabled = enabled
        findViewById<Button>(id.androidButton).isEnabled = enabled
        findViewById<Button>(id.iosButton).isEnabled = enabled
        findViewById<Button>(id.linuxButton).isEnabled = enabled
        findViewById<Button>(id.macButton).isEnabled = enabled
        findViewById<Button>(id.windowsButton).isEnabled = enabled
        findViewById<CheckBox>(id.sendFolderCheckBox).isEnabled = enabled

        findViewById<Button>(id.startButton).isInvisible = !enabled
        findViewById<Button>(id.cancelButton).isInvisible = enabled

        findViewById<TextView>(id.aboutButton).isClickable = enabled
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putString("output", outputBox.text.toString())
        val modeGroup = findViewById<MaterialButtonToggleGroup>(id.modeGroup)
        val modeIndex = when (modeGroup.checkedButtonId) {
            id.sendButton -> 1
            id.receiveButton -> 2
            else -> 0
        }
        outState.putInt("mode", modeIndex)
        val peerGroup = findViewById<MaterialButtonToggleGroup>(id.peerGroup)
        val peerIndex = when (peerGroup.checkedButtonId) {
            id.androidButton -> 1
            id.iosButton -> 2
            id.linuxButton -> 3
            id.macButton -> 4
            id.windowsButton -> 5
            else -> 0
        }
        outState.putInt("peer", peerIndex)
        val sendFolderCheckBox = findViewById<CheckBox>(id.sendFolderCheckBox)
        outState.putBoolean("sendFolderChecked", sendFolderCheckBox.isChecked)
        outState.putBoolean("sendFolderVisible", sendFolderCheckBox.isVisible)
        val transferRunning = !findViewById<Button>(id.startButton).isVisible
        outState.putBoolean("transferRunning", transferRunning)
        val progressBarValue = findViewById<ProgressBar>(id.progressBar).progress
        outState.putInt("progress", progressBarValue)
    }

    override fun onRestoreInstanceState(savedInstanceState: Bundle) {
        super.onRestoreInstanceState(savedInstanceState)
        outputBox.text = savedInstanceState.getString("output")
        val modeGroup = findViewById<MaterialButtonToggleGroup>(id.modeGroup)
        when (savedInstanceState.getInt("mode")) {
            1 -> modeGroup.check(id.sendButton)
            2 -> modeGroup.check(id.receiveButton)
        }
        val peerGroup = findViewById<MaterialButtonToggleGroup>(id.peerGroup)
        when (savedInstanceState.getInt("peer")) {
            1 -> peerGroup.check(id.androidButton)
            2 -> peerGroup.check(id.iosButton)
            3 -> peerGroup.check(id.linuxButton)
            4 -> peerGroup.check(id.macButton)
            5 -> peerGroup.check(id.windowsButton)
        }
        val sendFolderCheckBox = findViewById<CheckBox>(id.sendFolderCheckBox)
        sendFolderCheckBox.isChecked = savedInstanceState.getBoolean("sendFolderChecked")
        sendFolderCheckBox.isVisible = savedInstanceState.getBoolean("sendFolderVisible")
        val transferRunning = savedInstanceState.getBoolean("transferRunning")
        toggleUI(!transferRunning)
        if (transferRunning) {
            viewModel.qrBitmap?.let {
                findViewById<ImageView>(id.qrCodeView).setImageBitmap(it)
            }
        }
        findViewById<ProgressBar>(id.progressBar).progress = savedInstanceState.getInt("progress")
    }
}

// TODO:
// transfer "completing" if receiving end quit?
// check !!s
// test what happens if wifi is turned off - done. hotspot still runs, not sure about joining.
// if hotspot already in use, don't request again - hit start transfer twice - not a problem because of cancel button/ui? and error is caught and transfer cleaned up in this case?
// don't show progress bar till transfer starts?

// https://developers.google.com/ml-kit/code-scanner
