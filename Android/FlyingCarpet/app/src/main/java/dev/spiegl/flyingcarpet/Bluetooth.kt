package dev.spiegl.flyingcarpet

import android.Manifest
import android.annotation.SuppressLint
import android.app.Application
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothDevice.BOND_BONDED
import android.bluetooth.BluetoothDevice.BOND_BONDING
import android.bluetooth.BluetoothDevice.BOND_NONE
import android.bluetooth.BluetoothDevice.EXTRA_BOND_STATE
import android.bluetooth.BluetoothDevice.EXTRA_DEVICE
import android.bluetooth.BluetoothDevice.EXTRA_PREVIOUS_BOND_STATE
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.BluetoothLeScanner
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.location.LocationManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.ParcelUuid
import android.util.Log
import androidx.core.app.ActivityCompat
import androidx.lifecycle.LiveData
import androidx.lifecycle.MutableLiveData
import java.util.UUID

val SERVICE_UUID: UUID = UUID.fromString("A70BF3CA-F708-4314-8A0E-5E37C259BE5C")
val OS_CHARACTERISTIC_UUID: UUID = UUID.fromString("BEE14848-CC55-4FDE-8E9D-2E0F9EC45946")
val SSID_CHARACTERISTIC_UUID: UUID = UUID.fromString("0D820768-A329-4ED4-8F53-BDF364EDAC75")
val PASSWORD_CHARACTERISTIC_UUID: UUID = UUID.fromString("E1FA8F66-CF88-4572-9527-D5125A2E0762")
const val NO_SSID = "NONE"

interface BluetoothDelegate {
    fun gotPeer(peerOS: String)
    fun gotSsid(ssid: String)
    fun gotPassword(password: String)
    fun connectToPeer()
    fun getWifiInfo(): Pair<String, String>
    fun outputText(msg: String)
    fun bluetoothFailed()
}

class Bluetooth(val application: Application, private val delegate: BluetoothDelegate): BluetoothDelegate by delegate {

    lateinit var bluetoothManager: BluetoothManager
    lateinit var bluetoothGattServer: BluetoothGattServer
    lateinit var service: BluetoothGattService
    lateinit var bluetoothLeScanner: BluetoothLeScanner
    var bluetoothReceiver = BluetoothReceiver(application, null, delegate)
    var active = false

    // keeping these values here to stream wifiInfo over bluetooth since max packet size is 20
    // var wifiInfo = byteArrayOf()
    // var cursor = 0

    private var _status = MutableLiveData<Boolean>()
    val status: LiveData<Boolean>
        get() = _status

    fun stop(application: Context) {
        // Per-transfer state, so clear it at the per-transfer teardown rather than only in
        // scan(): scan() runs for the central role alone, and a peripheral transfer following
        // one that had completed its exchange would inherit `true` and spend the whole transfer
        // ignoring genuine Bluetooth failures. Before the permission gate below because this is
        // just a flag — nothing here needs a permission we might not have.
        bluetoothReceiver.exchangeComplete = false
        // Same reasoning for `bonded`: it means "this transfer's post-bond connection has been
        // opened", and it was never cleared anywhere, so the first fresh pairing in an app
        // session disarmed the post-bond connectGatt — the connection that reliably completes
        // the exchange — for every later fresh pairing. And for `result`: a stale scan result
        // left here would let an unrelated bond event connect to the previous transfer's peer.
        bluetoothReceiver.bonded = false
        bluetoothReceiver.result = null
        // From here until the next scan()/advertise(), no transfer owns the BLE stack, and
        // any callback that still arrives — ours or the peer's teardown — must read as
        // noise, not as a failure that flips the Bluetooth switch off (see bluetoothFailed
        // in MainViewModel). Set before the closes below so their own callbacks are covered.
        bluetoothReceiver.tearingDown = true
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
            && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_SCAN) != PackageManager.PERMISSION_GRANTED)
        {
            return
        }
        _status.postValue(false)
        // central: disconnect AND close every client connection this transfer opened.
        // Nulling the reference alone left the underlying BluetoothGatt connected with its
        // callback registered, so it would reconnect and re-run the whole OS exchange after
        // the transfer had ended — and closing only `bluetoothGatt` (the most recently
        // connected client) left the *other* one of the deliberately coexisting pre-bond and
        // post-bond pair registered and holding the ACL. Confirmed 2026-07-25, iOS→Android:
        // the leftover pre-bond client received iOS's teardown Service Changed right after
        // this method had cleared exchangeComplete, re-discovered, found the service gone,
        // and turned the Bluetooth switch off via bluetoothFailed().
        // Stop on the stored scanner, not a freshly fetched one: stopScan() matches the callback
        // against the instance that started the scan. Guard on adapter state instead, because
        // unlike stopAdvertising() below, stopScan() throws IllegalStateException("BT Adapter is
        // not turned ON") when Bluetooth has been switched off since the scan started; the catch
        // covers it going off between the check and the call.
        if (this::bluetoothLeScanner.isInitialized && bluetoothManager.adapter?.isEnabled == true) {
            try {
                bluetoothLeScanner.stopScan(leScanCallback)
            } catch (e: IllegalStateException) {
                Log.i("Bluetooth", "Bluetooth turned off during teardown: $e")
            }
        }
        bluetoothReceiver.closeAllConnections()
        // peripheral. adapter and bluetoothLeAdvertiser are null when Bluetooth is off or
        // unsupported — a user flipping Bluetooth off mid-transfer must not crash teardown.
        if (this::bluetoothManager.isInitialized) {
            bluetoothManager.adapter?.bluetoothLeAdvertiser?.stopAdvertising(advertiseCallback)
        }
        // Close the GATT server, not just clearServices(): an open server stays connectable
        // between transfers, so a peer (e.g. iOS starting its next transfer before this
        // device does) can reconnect and drive the exchange again. Reopen a fresh server so
        // the next transfer is ready with no client connections carried over. initializePeripheral
        // closes the old server before opening the new one and re-adds the service.
        if (this::bluetoothGattServer.isInitialized) {
            initializePeripheral(application)
        }
    }

    // peripheral

    fun initializePeripheral(application: Context): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
            && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED)
        {
            return false
        }
        if (bluetoothManager.adapter == null) {
            return false
        }

        // close any server from a previous transfer (or a previous onResume) before opening
        // a new one, so servers and their attached client connections don't accumulate
        if (this::bluetoothGattServer.isInitialized) {
            bluetoothGattServer.close()
        }

        // open server, create service
        bluetoothGattServer = bluetoothManager.openGattServer(application, serverCallback) ?: return false
        service = BluetoothGattService(SERVICE_UUID, BluetoothGattService.SERVICE_TYPE_PRIMARY)

        // add characteristics to service
        for (characteristicUuid in arrayOf(OS_CHARACTERISTIC_UUID, SSID_CHARACTERISTIC_UUID, PASSWORD_CHARACTERISTIC_UUID)) {
            val characteristic = BluetoothGattCharacteristic(
                characteristicUuid,
                BluetoothGattCharacteristic.PROPERTY_READ or BluetoothGattCharacteristic.PROPERTY_WRITE,
                BluetoothGattCharacteristic.PERMISSION_READ_ENCRYPTED_MITM or BluetoothGattCharacteristic.PERMISSION_WRITE_ENCRYPTED_MITM,
            )
            service.addCharacteristic(characteristic)
        }

        // add service to server
        bluetoothGattServer.addService(service)
        return true
    }

    private val serverCallback = object : BluetoothGattServerCallback() {
        @SuppressLint("MissingPermission")
        override fun onConnectionStateChange(device: BluetoothDevice?, status: Int, newState: Int) {
            Log.i("Bluetooth", "In serverCallback")
            super.onConnectionStateChange(device, status, newState)
            if (newState == BluetoothProfile.STATE_CONNECTED) {
                outputText("Device connected")
                // null if Bluetooth was switched off between the connect and this callback
                bluetoothManager.adapter?.bluetoothLeAdvertiser?.stopAdvertising(advertiseCallback)
                outputText("Stopped advertising")
            } else {
                outputText("Device disconnected")
            }
        }

        override fun onCharacteristicReadRequest(
            device: BluetoothDevice?,
            requestId: Int,
            offset: Int,
            characteristic: BluetoothGattCharacteristic?
        ) {
            super.onCharacteristicReadRequest(device, requestId, offset, characteristic)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED)
            {
                return
            }
            // device is nullable as of the SDK 37 stubs, and sendResponse requires it non-null
            if (device == null || characteristic == null) {
                return
            }
            when (characteristic.uuid) {
                // tell peer we're android
                OS_CHARACTERISTIC_UUID -> {
                    bluetoothGattServer.sendResponse(
                        device, requestId, BluetoothGatt.GATT_SUCCESS, 0, "android".toByteArray()
                    )
                }
                // if we've started wifi hotspot, this will send the details. if not, it will send a blank string and the peer will need to wait and try again
                SSID_CHARACTERISTIC_UUID -> {
                    val (ssid, _) = getWifiInfo()
                    bluetoothGattServer.sendResponse(
                        device, requestId, BluetoothGatt.GATT_SUCCESS, 0, ssid.toByteArray()
                    )
                }
                PASSWORD_CHARACTERISTIC_UUID -> {
                    val (_, password) = getWifiInfo()
                    bluetoothGattServer.sendResponse(
                        device, requestId, BluetoothGatt.GATT_SUCCESS, 0, password.toByteArray()
                    )
                }
                else -> {
                    outputText("Invalid characteristic")
                    bluetoothGattServer.sendResponse(
                        device,
                        requestId,
                        BluetoothGatt.GATT_REQUEST_NOT_SUPPORTED,
                        0,
                        null
                    )
                    return
                }
            }
        }

        override fun onCharacteristicWriteRequest(
            device: BluetoothDevice?,
            requestId: Int,
            characteristic: BluetoothGattCharacteristic?,
            preparedWrite: Boolean,
            responseNeeded: Boolean,
            offset: Int,
            value: ByteArray?
        ) {
            super.onCharacteristicWriteRequest(
                device,
                requestId,
                characteristic,
                preparedWrite,
                responseNeeded,
                offset,
                value
            )

            Log.i("Bluetooth", "Central peer wrote something: \"${value?.toString(Charsets.UTF_8)}\"")
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED)
            {
                return
            }
            if (device == null || characteristic == null) {
                return
            }
            when (characteristic.uuid) {
                OS_CHARACTERISTIC_UUID -> {
                    // now we know peer's OS
                    // thought we had to figure out hosting and connect here, but that doesn't
                    // happen till central writes wifi info
                    value?.let { gotPeer(it.toString(Charsets.UTF_8)) }
                }
                SSID_CHARACTERISTIC_UUID -> {
                    // central has written ssid to us as peripheral. if they wrote an ssid, we need to store it.
                    // if they didn't, we don't need to do anything, and just wait for them to write the password,
                    // at which point we can calculate the ssid and key.
                    if (value != null) {
                        gotSsid(value.toString(Charsets.UTF_8))
                    }
                }
                PASSWORD_CHARACTERISTIC_UUID -> {
                    if (value != null) {
                        gotPassword(value.toString(Charsets.UTF_8))
                    }
                }
                else -> {
                    outputText("Invalid characteristic")
                    bluetoothGattServer.sendResponse(
                        device,
                        requestId,
                        BluetoothGatt.GATT_REQUEST_NOT_SUPPORTED,
                        0,
                        null
                    )
                    return
                }
            }
            bluetoothGattServer.sendResponse(
                device,
                requestId,
                BluetoothGatt.GATT_SUCCESS,
                0,
                null
            )
        }
    }

    fun advertise() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
            && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_ADVERTISE) != PackageManager.PERMISSION_GRANTED)
        {
            return
        }
        // new transfer (peripheral role): re-arm bluetoothFailed(), which stop() disarms —
        // the peripheral never calls scan(), so it must clear the flag here
        bluetoothReceiver.tearingDown = false
        // BluetoothLeAdvertiser. null when Bluetooth is off: report and fail the transfer
        // rather than crash — this used to be an unguarded platform-type dereference.
        val bluetoothLeAdvertiser = bluetoothManager.adapter?.bluetoothLeAdvertiser
        if (bluetoothLeAdvertiser == null) {
            outputText("Bluetooth advertiser unavailable. Is Bluetooth turned on?")
            active = false
            bluetoothFailed()
            return
        }
        val settingsBuilder = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_BALANCED)
            .setConnectable(true)
            .setTimeout(0)
            .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_HIGH)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            settingsBuilder.setDiscoverable(true)
        }
        val settings = settingsBuilder.build()

        val data = AdvertiseData.Builder()
            // adapter.name is nullable (and the adapter can vanish if Bluetooth turns off)
            .setIncludeDeviceName((bluetoothManager.adapter?.name?.length ?: Int.MAX_VALUE) <= 8)
            .setIncludeTxPowerLevel(false)
            .addServiceUuid(ParcelUuid(SERVICE_UUID))
            .build()
        bluetoothLeAdvertiser.startAdvertising(settings, data, advertiseCallback)
    }

    private val advertiseCallback = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings?) {
            super.onStartSuccess(settingsInEffect)
            _status.postValue(true)
            outputText("Advertiser started")
        }

        override fun onStartFailure(errorCode: Int) {
            super.onStartFailure(errorCode)
            outputText("Advertiser failed to start: $errorCode")
            active = false
            bluetoothFailed()
        }
    }

    // central

    fun initializeCentral(): Boolean {
        // adapter is null when Bluetooth is unsupported; check it before dereferencing
        // rather than after (the old order NPE'd on the adapter access itself)
        val scanner = bluetoothManager.adapter?.bluetoothLeScanner ?: return false
        bluetoothLeScanner = scanner
        return true
    }

    // The master toggle, not the permission: the app can hold ACCESS_FINE_LOCATION and still
    // get nothing back from a scan while location is switched off system-wide.
    private fun locationEnabled(): Boolean {
        val locationManager =
            application.getSystemService(Context.LOCATION_SERVICE) as? LocationManager
        return locationManager?.isLocationEnabled ?: true
    }

    fun scan() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
            && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_SCAN) != PackageManager.PERMISSION_GRANTED)
        {
            outputText("Missing permission BLUETOOTH_SCAN")
            return
        }
        // Before API 31 a scan needs the system location toggle on, not just the location
        // permission, and with it off startScan() reports success and delivers nothing: no
        // results, no onScanFailed, so the peer is simply never found. API 31+ is exempt via
        // neverForLocation on BLUETOOTH_SCAN, and telling those users to switch location on
        // would be asking for something the app doesn't need.
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S && !locationEnabled()) {
            outputText(
                "Location is turned off. This version of Android requires it to find Bluetooth " +
                    "devices. Turn it on, or turn Bluetooth off here and use the QR code or " +
                    "password instead."
            )
        }
        val scanFilter = ScanFilter.Builder()
            .setServiceUuid(ParcelUuid(SERVICE_UUID))
            .build()
        val scanSettings = ScanSettings.Builder()
            // this was actually the culprit
            // .setLegacy(false)
            .build()
        // new transfer: allow the credential exchange (and its retry) to run again, let
        // this transfer's pairing (if one happens) open its own post-bond connection, and
        // re-arm bluetoothFailed() — this transfer's BLE events are real again
        bluetoothReceiver.exchangeComplete = false
        bluetoothReceiver.bonded = false
        bluetoothReceiver.tearingDown = false
        bluetoothLeScanner.startScan(listOf(scanFilter), scanSettings, leScanCallback)
        _status.postValue(true)
        outputText("Scanning for Bluetooth peripherals...")
    }

    private val leScanCallback = object : ScanCallback() {
        // this is called when we've scanned for a peripheral and found it. this calls createBond(),
        // and once the bonding process is complete, Android will send us the ACTION_BOND_STATE_CHANGED
        // event and we'll resume in BluetoothReceiver, which will discover services, then characteristics,
        // and store those in itself.
        override fun onScanResult(callbackType: Int, result: ScanResult?) {
            super.onScanResult(callbackType, result)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_SCAN) != PackageManager.PERMISSION_GRANTED)
            {
                outputText("Missing permission BLUETOOTH_SCAN")
                return
            }
            if (result != null) {
                if (bluetoothReceiver.waitingForConnection) {
                    outputText("Found device: ${result.device}")
                    bluetoothReceiver.waitingForConnection = false
                    bluetoothLeScanner.stopScan(this)
                    outputText("Stopped scanning")
                    //                address = result.device.address
                    bluetoothReceiver.result = result

//                    if (result.device.bondState == BOND_BONDED) {
                    bluetoothReceiver.trackConnection(result.device.connectGatt(
                        application.applicationContext,
                        false,
                        bluetoothReceiver.gattCallback,
                        BluetoothDevice.TRANSPORT_LE,
                    ))
                    Log.i("Bluetooth", "Called connectGatt()")
//                    } else {
//                        result.device.createBond()
//                        outputText("Called createBond()")
//                    }
                } else {
//                    outputText("Connected but not waiting for connection")
                }
            }
        }

        override fun onScanFailed(errorCode: Int) {
            Log.e("Bluetooth", "Scan failed: $errorCode")
            super.onScanFailed(errorCode)
            active = false
            bluetoothFailed()
        }
    }

    // this class receives the bluetooth bonded events
    // TODO: rename?
    class BluetoothReceiver(
        private val application: Application,
        var result: ScanResult?,
        private val delegate: BluetoothDelegate,
    ): BroadcastReceiver(), BluetoothDelegate by delegate {

        private var peerDevice: BluetoothDevice? = null
        var bluetoothGatt: BluetoothGatt? = null
        var osCharacteristic: BluetoothGattCharacteristic? = null
        var ssidCharacteristic: BluetoothGattCharacteristic? = null
        var passwordCharacteristic: BluetoothGattCharacteristic? = null
        var waitingForConnection = false
        // "This transfer's post-bond connection has been opened." Cleared in scan() and
        // stop(), like exchangeComplete: left latched, the first fresh pairing in an app
        // session would suppress the post-bond connectGatt for every later fresh pairing.
        var bonded = false
        // Set once the credential exchange has actually completed. Gates the post-bond
        // connection's replay: the replay must stay available as a retry until the exchange
        // succeeds once, then be suppressed so a reconnect doesn't re-run read-OS → write-OS →
        // connectToPeer against a transfer already in progress. Set only *after* success (not on
        // connect, which was the exchangeStarted mistake), so it can only ever remove a redundant
        // replay, never a needed retry. Also gates bluetoothFailed(), since the peer's teardown
        // arrives after this point and is indistinguishable from a failure.
        //
        // It must be set for **every role that reaches those guards**, which is the mistake worth
        // remembering: it was originally set only in connectToPeer()'s isHosting() branch, so a
        // *joining* device — Android receiving from Linux or Windows, an entirely ordinary
        // configuration — ran with all of them disarmed. Two writers now cover the four role
        // axes: connectToPeer() for the host, gotPassword() for either kind of joiner.
        // Cleared in scan() and, for roles that never scan, in stop().
        var exchangeComplete = false

        // True from stop() until the next scan() or advertise() — i.e. whenever no transfer
        // owns the BLE stack. Gates bluetoothFailed() (MainViewModel) the same way
        // exchangeComplete does, but for the window *between* transfers, which
        // exchangeComplete can't cover because stop() clears it: between transfers a late
        // BLE callback is a log line, never a reason to flip the Bluetooth switch off.
        // Starts true because no transfer is active until one starts.
        var tearingDown = true

        // Every GATT client this transfer opened, in the order opened. The pre-bond and
        // post-bond connections deliberately coexist (see onConnectionStateChange), and
        // `bluetoothGatt` only tracks the most recently connected one — so a teardown that
        // closes only `bluetoothGatt` strands the other client, still registered and still
        // holding the ACL between transfers (law 9 in docs/bluetooth-field-guide.md).
        private val openConnections = mutableListOf<BluetoothGatt>()

        fun trackConnection(gatt: BluetoothGatt?) {
            if (gatt != null) {
                synchronized(openConnections) { openConnections.add(gatt) }
            }
        }

        private fun untrackConnection(gatt: BluetoothGatt) {
            synchronized(openConnections) { openConnections.remove(gatt) }
        }

        // Close every client, not just the last-connected one — callbacks stop after
        // close(), which is what makes stop() actually final.
        @SuppressLint("MissingPermission")
        fun closeAllConnections() {
            val toClose = synchronized(openConnections) {
                val copy = openConnections.toList()
                openConnections.clear()
                copy
            }
            for (gatt in toClose) {
                gatt.disconnect()
                gatt.close()
            }
            bluetoothGatt = null
        }

        // For delays that used to be Thread.sleep() on the GATT binder thread: sleeping
        // there stalls every other callback behind it (Apple's equivalent was converted to
        // asyncAfter for the same reason), so schedule instead.
        private val handler = Handler(Looper.getMainLooper())

        // True from the moment discoverServices() is accepted until onServicesDiscovered fires
        // for it. Two call sites start a discovery — onConnectionStateChange after its settle,
        // and onServiceChanged when the peer's database changes — and nothing stopped them
        // overlapping. A peripheral that registers its service right as we connect makes them
        // overlap every time: observed 2026-07-25 on Linux→Android, two discoveries completing
        // 13 ms apart, each calling read(OS), the second silently dropped by the busy GATT
        // queue. Both chains were identical so it didn't matter, but two concurrent walks of
        // read-OS → write-OS → connectToPeer is not a state this code reasons about.
        private var discoveryOutstanding = false

        // Single door for discoverServices(), so both call sites get the overlap guard and
        // neither can ignore the return value. discoverServices() reports "busy" the same way
        // readCharacteristic() does — a false return, no callback, nothing logged — and the
        // discovery is what produces the characteristics, so dropping one silently strands the
        // transfer with an empty log.
        private fun startDiscovery(gatt: BluetoothGatt, reason: String) {
            if (discoveryOutstanding) {
                Log.i("Bluetooth", "Discovery already outstanding; not starting another ($reason)")
                return
            }
            if (!gatt.discoverServices()) {
                outputText("Could not start Bluetooth service discovery ($reason).")
                return
            }
            discoveryOutstanding = true
        }

        val gattCallback = object : BluetoothGattCallback() {
            // this is called when we as central have read a characteristic from the peer's peripheral
            override fun onCharacteristicRead(
                gatt: BluetoothGatt,
                characteristic: BluetoothGattCharacteristic,
                value: ByteArray,
                status: Int
            ) {
                super.onCharacteristicRead(gatt, characteristic, value, status)
                if (status != BluetoothGatt.GATT_SUCCESS) {
                    // without this, a failed read (e.g. after declined pairing) stalls the
                    // transfer, or propagates an empty value as the peer's OS/SSID/password
                    outputText("Failed to read Bluetooth characteristic (status $status).")
                    bluetoothFailed()
                    return
                }
                val stringRepresentation = value.toString(Charsets.UTF_8)
                Log.i("Bluetooth", "Read characteristic: $stringRepresentation")
                when (characteristic.uuid) {
                    OS_CHARACTERISTIC_UUID -> {
                        gotPeer(value.toString(Charsets.UTF_8))
                    }
                    SSID_CHARACTERISTIC_UUID -> {
                        val ssid = value.toString(Charsets.UTF_8)
                        if (ssid == "" || ssid == NO_SSID) {
                            // "" is an Android host whose hotspot isn't up yet; NO_SSID is a
                            // Windows host whose main thread hasn't generated credentials yet
                            // (our read can race it right after the OS exchange). Either way
                            // the credentials don't exist yet — wait a second and read again,
                            // which loops us back here. NO_SSID used to fall through as a
                            // final answer, which joined a hotspot derived from an empty
                            // password while the host waited forever for a real SSID read.
                            outputText("Could not read peer's WiFi characteristic. trying again...")
                            handler.postDelayed({ read(SSID_CHARACTERISTIC_UUID) }, 1000)
                            return
                        }
                        gotSsid(ssid)
                        // doing this here instead of in gotSsid because if peripheral had SSID
                        // written to it, we wouldn't need to call read
                        // we read the SSID, now read the password.
                        read(PASSWORD_CHARACTERISTIC_UUID)
                    }
                    PASSWORD_CHARACTERISTIC_UUID -> gotPassword(value.toString(Charsets.UTF_8))
                }
            }

            // this is called when we as central have written a characteristic to the peripheral
            override fun onCharacteristicWrite(
                gatt: BluetoothGatt?,
                characteristic: BluetoothGattCharacteristic?,
                status: Int
            ) {
                super.onCharacteristicWrite(gatt, characteristic, status)
                if (status != BluetoothGatt.GATT_SUCCESS) {
                    outputText("Failed to write Bluetooth characteristic (status $status).")
                    bluetoothFailed()
                    return
                }
                when (characteristic?.uuid) {
                    OS_CHARACTERISTIC_UUID -> {
                        outputText("Wrote OS to peer")
                        connectToPeer()
                    }
                    SSID_CHARACTERISTIC_UUID -> {
                        outputText("Wrote SSID to peer")
                        val (_, password) = getWifiInfo()
                        // outputText("Fetched password = $password")
                        write(PASSWORD_CHARACTERISTIC_UUID, password.toByteArray())
                    }
                    PASSWORD_CHARACTERISTIC_UUID -> {
                        outputText("Wrote password to peer")
                        // we told the peripheral the password, now just have to wait for them to join the hotspot
                    }
                }
            }

            override fun onServicesDiscovered(gatt: BluetoothGatt?, status: Int) {
                // before the permission gate: the discovery finished either way, and latching
                // this flag on would suppress every later re-discovery on this connection
                discoveryOutstanding = false
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                    && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED)
                {
                    return
                }
                super.onServicesDiscovered(gatt, status)
                // Every exit below used to `return` silently, leaving the transfer waiting
                // forever for a credential exchange that would never happen — three of them
                // without printing anything at all. Linux errors, Windows retries then
                // errors, and Apple cleans up; Android was the only platform that hung. See
                // docs/bluetooth-field-guide.md.
                if (status != BluetoothGatt.GATT_SUCCESS) {
                    outputText("Bluetooth service discovery failed with status $status.")
                    bluetoothFailed()
                    return
                }
                if (gatt == null) {
                    outputText("Bluetooth service discovery returned no GATT client.")
                    bluetoothFailed()
                    return
                }
                // Post-exchange this is the peer's teardown being observed, not progress worth
                // reporting: BLE has nothing left to contribute and the user is watching for
                // the Wi-Fi join. Keep it in logcat, out of the transfer output.
                if (exchangeComplete || tearingDown) {
                    Log.i("Bluetooth", "Discovered ${gatt.services.size} services")
                } else {
                    outputText("Discovered ${gatt.services.size} services")
                }
                val service = gatt.getService(SERVICE_UUID)
                if (service == null) {
                    // Android caches the GATT database for bonded devices, and every peripheral
                    // removes its service at teardown, so a stale cache lands exactly here and
                    // the unpair advice is the right remedy.
                    //
                    // *After* the credential exchange it means the opposite: the peer removed
                    // its service on purpose, the transfer has already moved to Wi-Fi, and
                    // nothing is wrong. bluetoothFailed() has been gated on exchangeComplete for
                    // that case, but the gate is downstream — this message printed first and
                    // unconditionally, so the benign teardown still announced a failure, and it
                    // landed in the gap between "Joining <ssid>" and the hotspot association
                    // where the user has nothing else to look at. Reported as looking like a
                    // failed transfer that then succeeded anyway.
                    if (exchangeComplete || tearingDown) {
                        Log.i(
                            "Bluetooth",
                            "Peer removed its Flying Carpet service after the exchange; expected"
                        )
                    } else {
                        outputText(
                            "Did not find the Flying Carpet service on the peer. If the other " +
                            "device has started its transfer, try unpairing the two devices from " +
                            "each other and running the transfer again."
                        )
                    }
                    bluetoothFailed()
                    return
                }
                val os = service.getCharacteristic(OS_CHARACTERISTIC_UUID)
                val ssid = service.getCharacteristic(SSID_CHARACTERISTIC_UUID)
                val password = service.getCharacteristic(PASSWORD_CHARACTERISTIC_UUID)
                if (os == null || ssid == null || password == null) {
                    outputText(
                        "Peer's Flying Carpet service is missing characteristics " +
                        "(os: ${os != null}, ssid: ${ssid != null}, password: ${password != null})."
                    )
                    bluetoothFailed()
                    return
                }
                osCharacteristic = os
                ssidCharacteristic = ssid
                passwordCharacteristic = password
                read(OS_CHARACTERISTIC_UUID)
            }

            override fun onServiceChanged(gatt: BluetoothGatt) {
                super.onServiceChanged(gatt)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                    && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED)
                {
                    return
                }
                outputText("Services changed")
                // This is the peer telling us its GATT database changed, and it is the only
                // signal Android gives us that its cache for a *bonded* device is stale.
                // Every peripheral removes its Flying Carpet service when a transfer ends and
                // re-adds it on the next one, so without re-discovering here a bonded central
                // can keep serving a snapshot that has no Flying Carpet service in it.
                //
                // The TODO this replaces asked whether enabling it causes problems. It does,
                // if left ungated: onServicesDiscovered re-reads the characteristics and calls
                // read(OS_CHARACTERISTIC_UUID), which restarts the credential exchange. That
                // is the same re-entrancy hazard onConnectionStateChange already guards with
                // exchangeComplete, so guard it the same way — before the exchange finishes we
                // want the re-discovery, after it the transfer is on TCP and this is pure
                // interference.
                if (exchangeComplete) {
                    Log.i("Bluetooth", "Ignoring service change; credential exchange already complete")
                    return
                }
                // Between transfers this is the peer's teardown removing its service —
                // re-discovering would find the service gone and fail a transfer that no
                // longer exists. This is the callback that flipped the Bluetooth switch off
                // on 2026-07-25 (iOS→Android), delivered to a leftover client after stop()
                // had cleared exchangeComplete.
                if (tearingDown) {
                    Log.i("Bluetooth", "Ignoring service change after teardown")
                    return
                }
                startDiscovery(gatt, "service change")
            }

            override fun onConnectionStateChange(
                gatt: BluetoothGatt?,
                status: Int,
                newState: Int
            ) {
                super.onConnectionStateChange(gatt, status, newState)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                    && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED)
                {
                    return
                }
                if (newState == BluetoothProfile.STATE_CONNECTED) {
                    bluetoothGatt = gatt
                    // a fresh connection has no discovery outstanding on it, whatever the
                    // previous one left behind
                    discoveryOutstanding = false
                    outputText("Connected")
                    // Both GATT connections we open — the autoConnect=false one from
                    // onScanResult and the autoConnect=true one the bond receiver opens after
                    // pairing — run discoverServices() and replay read-OS → write-OS. That
                    // repetition is deliberate: the first connection's encrypted read only
                    // *triggers* pairing, and its write-back can be lost while the link is
                    // still bonding, so the post-bond connection is the one that reliably
                    // completes the exchange.
                    //
                    // Once the exchange has actually completed, though, further replays are
                    // pure waste — the transfer is over TCP now — and the autoConnect=true
                    // link re-establishing mid-transfer would otherwise re-run the whole chain.
                    // So skip discovery once the exchange is done; until then, keep retrying.
                    if (exchangeComplete || tearingDown) {
                        Log.i("Bluetooth", "Skipping rediscovery; credential exchange complete or transfer torn down")
                        return
                    }
                    // 1600ms is Nordic's Android-BLE-Library constant: on a bonded device,
                    // wait ~1.6s before discoverServices() so the Service Changed indication
                    // and key exchange land first, or you enumerate a stale GATT database. We
                    // bond and never invalidate the cache. Possibly removable (Nordic say it
                    // was an Android 6 problem; minSdk is 29) — see
                    // docs/hardcoded-delays-audit.md before touching it.
                    //
                    // Scheduled rather than slept: this callback runs on the GATT binder
                    // thread, and sleeping there stalls every other callback behind it. Only
                    // fire if this is still the live connection, or a link that dropped during
                    // the delay produces a spurious "could not start discovery".
                    gatt?.let {
                        handler.postDelayed({
                            // Re-check on fire, not just at schedule time: the exchange can
                            // finish inside the 1600ms (~900ms observed) and the peer drops its
                            // service when it does, so a stale discovery lands in the
                            // service-missing branch and printed the "try unpairing" advice
                            // mid-join.
                            if (exchangeComplete || tearingDown) {
                                Log.i(
                                    "Bluetooth",
                                    "Skipping scheduled discovery; exchange completed during the settle delay"
                                )
                                return@postDelayed
                            }
                            if (bluetoothGatt === it) {
                                startDiscovery(it, "connected")
                            }
                        }, 1600)
                    }
                } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                    // This was a bare Log.i that ignored `status` entirely — the last silent
                    // failure on the Android central path. 6b29695 instrumented every exit
                    // from onServicesDiscovered, but a connect that never succeeds never
                    // reaches it, so the UI simply went quiet right after "Stopped scanning"
                    // and the status code that names the cause was dropped on the floor.
                    // Observed 2026-07-25, Linux->Android.
                    //
                    // close() is not optional. Android leaks the underlying client if it is
                    // not called after a disconnect, and connectGatt() starts failing with
                    // status 133 once enough have leaked. Nothing closed it here, and
                    // Bluetooth.stop() only closes `bluetoothGatt`, which a *failed* connect
                    // never assigns — so every failed attempt leaked one and the retries
                    // leaked more. That is what turns one transient error into a device that
                    // stays broken until the app is restarted.
                    val current = bluetoothGatt
                    gatt?.close()
                    gatt?.let { untrackConnection(it) }
                    if (current === gatt) {
                        bluetoothGatt = null
                        // a discovery on a link that is gone will never call back, so don't let
                        // it latch the guard on and block the next connection's discovery
                        discoveryOutstanding = false
                    }
                    when {
                        // The peer hangs up once it has our credentials and the transfer has
                        // moved to TCP. Expected — Linux now does exactly this.
                        exchangeComplete ->
                            Log.i("Bluetooth", "Disconnected after exchange (status $status)")
                        // Between stop() and the next transfer, disconnects are teardown
                        // fallout — ours or the peer's — never a failure.
                        tearingDown ->
                            Log.i("Bluetooth", "Disconnected during teardown (status $status)")
                        // Pairing in flight. The first connection's encrypted read only
                        // *triggers* bonding, and the link commonly drops doing it; the
                        // ACTION_BOND_STATE_CHANGED receiver then opens the connection that
                        // actually completes the exchange. A step, not a failure.
                        result?.device?.bondState == BOND_BONDING ->
                            Log.i("Bluetooth", "Disconnected while bonding (status $status)")
                        // An older connection dropping while a newer one is live. The two
                        // overlap deliberately after bonding (see the comment above), so only
                        // the live one is allowed to fail the transfer.
                        current != null && current !== gatt ->
                            Log.i("Bluetooth", "Stale connection dropped (status $status)")
                        else -> {
                            outputText("Bluetooth connection failed with status $status.")
                            if (status == 133) {
                                outputText(
                                    "Status 133 is Android's generic GATT failure. If it keeps " +
                                    "happening, restart Flying Carpet on this device; if it " +
                                    "still fails, unpair the two devices from each other."
                                )
                            }
                            bluetoothFailed()
                        }
                    }
                } else {
                    Log.i("Bluetooth", "New connection state: $newState")
                }
            }
        }

        // called when we get a bluetooth bonding event from the OS
        @SuppressLint("MissingPermission")
        override fun onReceive(context: Context?, intent: Intent?) {
            Log.i("Bluetooth", "Action: ${intent?.action}")
            peerDevice = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                intent?.getParcelableExtra(EXTRA_DEVICE, BluetoothDevice::class.java)
            } else {
                intent?.getParcelableExtra(EXTRA_DEVICE)
            }
            val bondState = intent?.getIntExtra(EXTRA_BOND_STATE, -1)
            if (bondState != BOND_BONDED) {
                // BONDING -> NONE means pairing failed or the user declined the system
                // pairing dialog. Without this, the transfer waits forever for
                // characteristic reads that will never happen. This receiver is registered
                // for the whole activity, so only react to our peer's bond events.
                val previousBondState = intent?.getIntExtra(EXTRA_PREVIOUS_BOND_STATE, -1)
                if (bondState == BOND_NONE && previousBondState == BOND_BONDING
                    && peerDevice != null && peerDevice?.address == result?.device?.address
                ) {
                    outputText("Bluetooth pairing failed or was declined.")
                    bluetoothFailed()
                } else {
                    Log.i("Bluetooth", "Not bonded")
                }
                return
            }
            // outputText("Device: $peerDevice")

            if (result == null) {
                Log.e("Bluetooth", "Received ACTION_BOND_STATE_CHANGED but do not have device result")
                return
            }
            // This receiver hears every bond event on the system, not just our peer's. A
            // headset bonding mid-transfer must not open (or use up) the post-bond
            // connection meant for the device we scanned.
            if (peerDevice?.address != result?.device?.address) {
                Log.i("Bluetooth", "Bond state change for a different device; ignoring")
                return
            }
            if (!bonded) {
                bonded = true
                trackConnection(result!!.device.connectGatt(
                    application.applicationContext,
                    true,
                    gattCallback,
                    // TRANSPORT_LE, never TRANSPORT_AUTO. Flying Carpet's GATT service exists
                    // only over LE, and this fires the instant bonding completes — exactly
                    // when cross-transport key derivation has minted BR/EDR keys alongside the
                    // LE ones. Letting the stack choose from a dual-transport bond is what
                    // made BlueZ page classic and fail with br-connection-canceled against a
                    // peer that serves no GATT there (docs/bluetooth-field-guide.md).
                    BluetoothDevice.TRANSPORT_LE,
                ))
            } else {
                Log.e("Bluetooth", "Received ACTION_BOND_STATE_CHANGED but already bonded")
            }
        }

        // Which characteristic a UUID refers to on the peer, once discovery has resolved them.
        private fun characteristicFor(characteristicUuid: UUID): BluetoothGattCharacteristic? {
            return when (characteristicUuid) {
                OS_CHARACTERISTIC_UUID -> osCharacteristic
                SSID_CHARACTERISTIC_UUID -> ssidCharacteristic
                PASSWORD_CHARACTERISTIC_UUID -> passwordCharacteristic
                else -> null
            }
        }

        // use to read peripheral's characteristic
        fun read(characteristicUuid: UUID) {
            // outputText("Reading $characteristicUuid")
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED)
            {
                outputText("No permission")
                return
            }
            // Three ways this used to do nothing whatsoever, none of them printing anything: a
            // null bluetoothGatt (the `?.` swallowed the entire call), a null characteristic (an
            // unknown UUID fell through the `when` with no else), and readCharacteristic()
            // returning false because the GATT queue was busy or the link was gone. Each read is
            // what triggers the callback that issues the next step, so any drop stalls the
            // exchange for good — and one was observed doing exactly that on 2026-07-25.
            //
            // Reported, not fatal. A false return is legitimately transient: the two GATT
            // connections after bonding deliberately coexist and can each be walking the
            // exchange, so one of them finding the queue busy is not grounds for killing a
            // transfer the other is about to complete.
            val gatt = bluetoothGatt
            val characteristic = characteristicFor(characteristicUuid)
            if (gatt == null) {
                outputText("Could not read $characteristicUuid: no Bluetooth connection.")
            } else if (characteristic == null) {
                outputText("Could not read $characteristicUuid: characteristic not discovered.")
            } else if (!gatt.readCharacteristic(characteristic)) {
                outputText("Bluetooth read of $characteristicUuid was rejected (queue busy or link dropped).")
            }
        }

        // private fun writeSinglePacket(characteristicUuid: UUID, value: ByteArray, waitForResponse: Boolean) {
        fun write(characteristicUuid: UUID, value: ByteArray) {
            // outputText("Writing to $characteristicUuid")
            // val writeType = if (waitForResponse) BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT else BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
            val writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
                && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED)
            {
                return
            }
            // Same three silent drops as read() above, plus a `characteristic!!` on the Tiramisu
            // path that would have thrown instead of reporting. Reported, not fatal, for the same
            // reason.
            val gatt = bluetoothGatt
            val characteristic = characteristicFor(characteristicUuid)
            if (gatt == null) {
                outputText("Could not write $characteristicUuid: no Bluetooth connection.")
                return
            }
            if (characteristic == null) {
                outputText("Could not write $characteristicUuid: characteristic not discovered.")
                return
            }
            val queued = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                gatt.writeCharacteristic(characteristic, value, writeType) == BluetoothStatusCodes.SUCCESS
            } else {
                characteristic.value = value
                characteristic.writeType = writeType
                @Suppress("DEPRECATION")
                gatt.writeCharacteristic(characteristic)
            }
            if (!queued) {
                outputText("Bluetooth write of $characteristicUuid was rejected (queue busy or link dropped).")
            }
        }

        // going to split ssid and password into separate characteristics to avoid having to implement streaming,
        // in the hope that android will never make hotspots with SSIDs or passwords longer than 20 characters
//        fun write(characteristicUuid: UUID, value: ByteArray) {
//            var cursor = 0
//            while (cursor < value.size) {
//                val chunk = value.slice(cursor until min(cursor + packetSize, value.size))
//                cursor += chunk.size
//                writeSinglePacket(characteristicUuid, chunk.toByteArray(), false)
//            }
//            writeSinglePacket(characteristicUuid, messageTerminator, true)
//        }
    }


}

