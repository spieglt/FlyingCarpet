package dev.spiegl.flyingcarpet

import android.Manifest
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.content.IntentFilter
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.util.Log
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
import androidx.appcompat.widget.SwitchCompat
import androidx.core.app.ActivityCompat
import androidx.core.view.isVisible
import androidx.documentfile.provider.DocumentFile
import androidx.lifecycle.ViewModelProvider
import com.google.android.material.button.MaterialButtonToggleGroup
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import dev.spiegl.flyingcarpet.R.id
import dev.spiegl.flyingcarpet.R.layout

class MainActivity : AppCompatActivity() {
    private lateinit var viewModel: MainViewModel
    private lateinit var outputBox: TextView
    private lateinit var progressBar: ProgressBar
    private lateinit var bluetoothRequestPermissionLauncher: ActivityResultLauncher<Array<String>>
    private lateinit var filePicker: ActivityResultLauncher<Array<String>>
    private lateinit var folderPicker: ActivityResultLauncher<Uri?>
    private lateinit var peerGroup: MaterialButtonToggleGroup
    private lateinit var peerInstruction: TextView
    private lateinit var bluetoothSwitch: SwitchCompat
    private lateinit var bluetoothIcon: ImageView

    private fun getFilePicker(): ActivityResultLauncher<Array<String>> {
        return registerForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
            viewModel.files = mutableListOf()
            viewModel.fileStreams = mutableListOf()
            viewModel.filePaths = mutableListOf()
            if (uris.isEmpty()) {
                viewModel.outputText("No files selected.")
                viewModel.cleanUpTransfer()
                return@registerForActivityResult
            }
            for (uri in uris) {
                val file = DocumentFile.fromSingleUri(applicationContext, uri)
                if (file != null) {
                    viewModel.files.add(file)
                } else {
                    viewModel.outputText("Could not open file")
                    viewModel.cleanUpTransfer()
                    return@registerForActivityResult
                }
                val stream = applicationContext.contentResolver.openInputStream(uri)
                if (stream != null) {
                    viewModel.fileStreams.add(stream)
                } else {
                    viewModel.outputText("Could not open file stream")
                    viewModel.cleanUpTransfer()
                    return@registerForActivityResult
                }
            }

            // if using bluetooth, start the process of exchanging OS and wifi information
            if (viewModel.bluetooth.active) {
                if (viewModel.mode == Mode.Sending) {
                    viewModel.bluetooth.advertise()
                } else if (viewModel.mode == Mode.Receiving) {
                    viewModel.bluetooth.scan()
                }
            } else {
                viewModel.connectToPeer()
            }
        }
    }

    private fun getFolderPicker(): ActivityResultLauncher<Uri?> {
        return registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
            uri?.let {
                if (viewModel.mode == Mode.Sending) {
                    viewModel.files = mutableListOf()
                    viewModel.fileStreams = mutableListOf()
                    viewModel.filePaths = mutableListOf()
                    val dir = DocumentFile.fromTreeUri(applicationContext, it) ?: run {
                        viewModel.outputText("Could not get DocumentFile from selected directory.")
                        viewModel.cleanUpTransfer()
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
                            viewModel.cleanUpTransfer()
                            return@registerForActivityResult
                        }
                    }
                    viewModel.sendDir = it
                } else {
                    viewModel.receiveDir = it
                }
                // if using bluetooth, start the process of exchanging OS and wifi information
                if (viewModel.bluetooth.active) {
                    if (viewModel.mode == Mode.Sending) {
                        viewModel.bluetooth.advertise()
                    } else if (viewModel.mode == Mode.Receiving) {
                        viewModel.bluetooth.scan()
                    }
                } else {
                    viewModel.connectToPeer()
                }
            } ?: run {
                viewModel.outputText("No folder selected.")
                viewModel.cleanUpTransfer()
                return@registerForActivityResult
            }
        }
    }

    private fun getRequestPermissionLauncher(): ActivityResultLauncher<String> {
        return registerForActivityResult(ActivityResultContracts.RequestPermission()) { isGranted: Boolean ->
            if (isGranted) {
                // Permission is granted. Continue the action or workflow in your app.
                viewModel.outputText("Permission granted.")
                // start hotspot here
                viewModel.startHotspot()
            } else {
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
                viewModel.cleanUpTransfer()
            }
        }
    }

    private fun getBarcodeLauncher(): ActivityResultLauncher<ScanOptions> {
        return registerForActivityResult(ScanContract()) { result ->
            if (result.contents == null) {
                viewModel.outputText("Scan cancelled, exiting transfer.")
                viewModel.cleanUpTransfer()
            } else {
                val ssidAndPassword = result.contents.split(';')
                if (ssidAndPassword.count() > 1) {
                    viewModel.ssid = ssidAndPassword[0]
                    viewModel.password = ssidAndPassword[1]
                    val (_, key) = getSsidAndKey(viewModel.password)
                    viewModel.key = key
                } else {
                    viewModel.password = ssidAndPassword[0]
                    val (ssid, key) = getSsidAndKey(viewModel.password)
                    viewModel.ssid = ssid
                    viewModel.key = key
                }
                // join hotspot
                viewModel.joinHotspot()
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(layout.activity_main)

        viewModel = ViewModelProvider(this)[MainViewModel::class.java]

        // set up file and folder pickers
        filePicker = getFilePicker()
        folderPicker = getFolderPicker()

        // set up permissions request
        viewModel.wifiManager = applicationContext.getSystemService(WIFI_SERVICE) as WifiManager
        viewModel.requestPermissionLauncher = getRequestPermissionLauncher()

        viewModel.barcodeLauncher = getBarcodeLauncher()
        viewModel.displayQrCode = ::displayQrCode
        viewModel.cleanUpUi = ::cleanUpUi

        peerGroup = findViewById(id.peerGroup)
        peerInstruction = findViewById(id.peerInstruction)
        outputBox = findViewById(id.outputBox)
        viewModel.output.observe(this) { msg ->
            outputBox.append(msg + '\n')
        }
        progressBar = findViewById(id.progressBar)
        viewModel.progressBar.observe(this) { value ->
            progressBar.progress = value
        }
        viewModel.transferFinished.observe(this) { finished ->
            // this was firing because when we started observing, we were running viewModel.cleanUpTransfer()
            // no matter what. and then _transferFinished was true. now initializing as false.
            if (finished) {
                viewModel.cleanUpTransfer()
            }
        }

        // set up bluetooth
        bluetoothOnCreate()

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
                    viewModel.cleanUpTransfer()
                    return@setOnClickListener
                }
            }

            // get peer
            val selectedPeer = peerGroup.checkedButtonId
            if (!viewModel.bluetooth.active) {
                this.viewModel.peer = when (selectedPeer) {
                    id.androidButton -> Peer.Android
                    id.iosButton -> Peer.iOS
                    id.linuxButton -> Peer.Linux
                    id.macButton -> Peer.macOS
                    id.windowsButton -> Peer.Windows
                    else -> {
                        viewModel.outputText("Must select operating system of other device.")
                        viewModel.cleanUpTransfer()
                        return@setOnClickListener
                    }
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
            viewModel.cleanUpTransfer()
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

    private fun cleanUpUi() {
        // toggle UI and replace icon
        runOnUiThread {
            requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
            toggleUI(true)
            val qrCode = findViewById<ImageView>(id.qrCodeView)
            val drawable = AppCompatResources.getDrawable(applicationContext, R.drawable.icon1024)
            qrCode.setImageDrawable(drawable)
        }
    }

    private fun displayQrCode(ssid: String, password: String) {
        if (viewModel.peer == Peer.iOS || viewModel.peer == Peer.Android) {
            // display qr code
            val qrCode = findViewById<ImageView>(id.qrCodeView)
            viewModel.qrBitmap = getQrCodeBitmap(ssid, password)
            qrCode.setImageBitmap(viewModel.qrBitmap)
        } else { // peer is macOS, because if windows or linux we wouldn't be hosting
            val alertFragment = Alert(ssid, password)
            alertFragment.show(supportFragmentManager, "alert")
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

        findViewById<Button>(id.startButton).isVisible = enabled
        findViewById<Button>(id.cancelButton).isVisible = !enabled

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

    // bluetooth

    private var permissions = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        arrayOf(
            Manifest.permission.ACCESS_COARSE_LOCATION,
            Manifest.permission.ACCESS_FINE_LOCATION,
            Manifest.permission.BLUETOOTH_ADVERTISE,
            Manifest.permission.BLUETOOTH_CONNECT,
            Manifest.permission.BLUETOOTH_SCAN,
        )
    } else {
        arrayOf(
            Manifest.permission.ACCESS_FINE_LOCATION,
            Manifest.permission.BLUETOOTH_ADMIN,
            Manifest.permission.BLUETOOTH,
        )
    }

    private fun checkForBluetoothPermissions(): Boolean {
        for (permission in permissions) {
            if (ActivityCompat.checkSelfPermission(this, permission) != PackageManager.PERMISSION_GRANTED) {
                viewModel.outputText("Missing permission: $permission")
                return false
            }
        }
        viewModel.outputText("All permissions granted")
        return true
    }

    private fun bluetoothOnCreate() {
        val bluetoothManager = getSystemService(BluetoothManager::class.java)
        viewModel.bluetooth.bluetoothManager = bluetoothManager

        bluetoothRequestPermissionLauncher = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { results: Map<String, Boolean> ->
            var allPermissionsGranted = true
            for (result in results) {
                viewModel.outputText("Have permission ${result.key}: ${result.value}")
                if (!result.value) {
                    allPermissionsGranted = false
                }
            }
            if (allPermissionsGranted) {
                viewModel.outputText("Bluetooth permissions granted")
            } else {
//                viewModel.outputText("To use Flying Carpet, either grant Bluetooth permissions to the app, or turn off the Use Bluetooth switch.")
                Log.e("Bluetooth", "To use Flying Carpet, either grant Bluetooth permissions to the app, or turn off the Use Bluetooth switch.")
                bluetoothSwitch.isChecked = false
//                if (initializeBluetooth()) {
//                    viewModel.outputText("Bluetooth initialized")
//                } else {
//                    viewModel.outputText("Device can't use Bluetooth")
//                    bluetoothSwitch.isChecked = false
//                    bluetoothSwitch.isEnabled = false
//                }
            }
        }

        bluetoothIcon = findViewById(id.bluetoothIcon)
        viewModel.bluetooth.status.observe(this) {
            bluetoothIcon.setBackgroundColor(Color.BLUE)
        }
        bluetoothSwitch = findViewById(id.bluetoothSwitch)
        bluetoothSwitch.setOnCheckedChangeListener { _, isChecked ->
            bluetoothIcon.isVisible = isChecked
            peerGroup.isVisible = !isChecked
            peerInstruction.isVisible = !isChecked
        }

        // register for bluetooth bonding events
        val filter = IntentFilter(BluetoothDevice.ACTION_BOND_STATE_CHANGED)
        registerReceiver(viewModel.bluetooth.bluetoothReceiver, filter)

        if (initializeBluetooth()) {
            viewModel.outputText("Bluetooth initialized")
        } else {
            viewModel.outputText("Device can't use Bluetooth")
            bluetoothSwitch.isChecked = false
            bluetoothSwitch.isEnabled = false
        }
    }

    private fun initializeBluetooth(): Boolean {
        if (!checkForBluetoothPermissions()) {
            Log.e("Bluetooth", "Missing permissions")
            bluetoothRequestPermissionLauncher.launch(permissions)
            return false
        }
        var initialized = false
        try {
            // TODO: undo after testing
            val initializedPeripheral = viewModel.bluetooth.initializePeripheral(this)
            val initializedCentral = viewModel.bluetooth.initializeCentral()
            if (!initializedPeripheral) {
                Log.e("Bluetooth", "Device cannot act as a Bluetooth peripheral")
            } else if (!initializedCentral) {
                Log.e("Bluetooth", "Device cannot act as a Bluetooth central")
            } else {
                initialized = true
            }
        } catch (e: Exception) {
            Log.e("Bluetooth", "Could not initialize Bluetooth: $e")
        }
        viewModel.bluetooth.active = initialized
        bluetoothSwitch.isChecked = initialized
        bluetoothSwitch.isEnabled = initialized
        bluetoothIcon.isVisible = initialized
        return initialized
    }
}

// TODO:
//   send mode before OS over bluetooth? necessary?
//   mutex needed for wifi info?
//   can't run advertiser more than once, have to quit app
//   bluetooth permissions messed up on launch
//   support if devices already bonded
//   text box scrolls up over cancel button
//   one permission check for all permissions?
//   bluetooth UI in landscape mode
//   bluetooth UI save/reload when screen rotated
//   bluetooth icon color change when scan/advertisement stops or starts: livedata?
//   transfer "completing" if receiving end quit?
//   check !!s
//   test what happens if wifi is turned off - done. hotspot still runs, not sure about joining.
//   if hotspot already in use, don't request again - hit start transfer twice - not a problem because of cancel button/ui? and error is caught and transfer cleaned up in this case?
//   don't show progress bar till transfer starts?

// https://developers.google.com/ml-kit/code-scanner
