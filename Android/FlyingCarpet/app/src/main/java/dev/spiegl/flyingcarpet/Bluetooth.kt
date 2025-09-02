package dev.spiegl.flyingcarpet

import android.Manifest
import android.annotation.SuppressLint
import android.app.Application
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothDevice.BOND_BONDED
import android.bluetooth.BluetoothDevice.EXTRA_BOND_STATE
import android.bluetooth.BluetoothDevice.EXTRA_DEVICE
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
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
import android.os.Build
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
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
            && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_SCAN) != PackageManager.PERMISSION_GRANTED)
        {
            return
        }
        _status.postValue(false)
        // central
        bluetoothLeScanner.stopScan(leScanCallback)
        bluetoothReceiver.bluetoothGatt = null
        // peripheral
        bluetoothManager.adapter.bluetoothLeAdvertiser.stopAdvertising(advertiseCallback)
        // this prevents android from sending twice? but disabling it leaves it advertising or offering services even after the stopAdvertising() above?
        // need to clear services and replace between transfers?
        // bluetoothGattServer.close()
        bluetoothGattServer.clearServices()
    }

    // peripheral

    fun initializePeripheral(application: Context): Boolean {
        if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
            return false
        }
        if (bluetoothManager.adapter == null) {
            return false
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
                val bluetoothLeAdvertiser = bluetoothManager.adapter.bluetoothLeAdvertiser
                bluetoothLeAdvertiser.stopAdvertising(advertiseCallback)
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
            if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
                return
            }
            if (characteristic == null) {
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
            if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
                return
            }
            if (characteristic == null) {
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
        if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_ADVERTISE) != PackageManager.PERMISSION_GRANTED) {
            return
        }
        // BluetoothLeAdvertiser
        val bluetoothLeAdvertiser = bluetoothManager.adapter.bluetoothLeAdvertiser
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
            .setIncludeDeviceName(bluetoothManager.adapter.name.length <= 8)
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
        if (bluetoothManager.adapter.bluetoothLeScanner == null) {
            return false
        }
        bluetoothLeScanner = bluetoothManager.adapter.bluetoothLeScanner
        return bluetoothManager.adapter != null
    }

    fun scan() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
            && ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_SCAN) != PackageManager.PERMISSION_GRANTED)
        {
            outputText("Missing permission BLUETOOTH_SCAN")
            return
        }
        val scanFilter = ScanFilter.Builder()
            .setServiceUuid(ParcelUuid(SERVICE_UUID))
            .build()
        val scanSettings = ScanSettings.Builder()
            // this was actually the culprit
            // .setLegacy(false)
            .build()
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
                    result.device.connectGatt(
                        application.applicationContext,
                        false,
                        bluetoothReceiver.gattCallback,
                        BluetoothDevice.TRANSPORT_LE,
                    )
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
        private var bonded = false

        val gattCallback = object : BluetoothGattCallback() {
            // this is called when we as central have read a characteristic from the peer's peripheral
            override fun onCharacteristicRead(
                gatt: BluetoothGatt,
                characteristic: BluetoothGattCharacteristic,
                value: ByteArray,
                status: Int
            ) {
                super.onCharacteristicRead(gatt, characteristic, value, status)
                val stringRepresentation = value.toString(Charsets.UTF_8)
                Log.i("Bluetooth", "Read characteristic: $stringRepresentation")
                when (characteristic.uuid) {
                    OS_CHARACTERISTIC_UUID -> {
                        gotPeer(value.toString(Charsets.UTF_8))
                    }
                    SSID_CHARACTERISTIC_UUID -> {
                        val ssid = value.toString(Charsets.UTF_8)
                        if (ssid == "") {
                            // peripheral hasn't stood up its hotspot yet, have to wait.
                            // kill a second, then read again, which will loop us back here.
                            outputText("Could not read peer's WiFi characteristic. trying again...")
                            Thread.sleep(1000)
                            read(SSID_CHARACTERISTIC_UUID)
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
                if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
                    return
                }
                super.onServicesDiscovered(gatt, status)
                outputText("Discovered services")
                for (service in gatt?.services!!) {
                    // outputText("Service: ${service.uuid}")
                }
                val service = gatt.getService(SERVICE_UUID)
                if (service == null) {
                    outputText("Did not find service")
//                    outputText("Trying to find services again")
//                    Thread.sleep(1000)
//                    gatt.discoverServices()
                    return
                }
                // outputText("Got service: $service")
                osCharacteristic = service.getCharacteristic(OS_CHARACTERISTIC_UUID) ?: return
                ssidCharacteristic = service.getCharacteristic(SSID_CHARACTERISTIC_UUID) ?: return
                passwordCharacteristic = service.getCharacteristic(PASSWORD_CHARACTERISTIC_UUID) ?: return
                // outputText("Got characteristics: $osCharacteristic, $ssidCharacteristic, $passwordCharacteristic")
                read(OS_CHARACTERISTIC_UUID)
            }

            override fun onServiceChanged(gatt: BluetoothGatt) {
                super.onServiceChanged(gatt)
                if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
                    return
                }
                outputText("Services changed")
                // TODO: should this be enabled? does it cause problems? https://developer.android.com/reference/android/bluetooth/BluetoothGattCallback#onServiceChanged(android.bluetooth.BluetoothGatt)
                // gatt.discoverServices()
            }

            override fun onConnectionStateChange(
                gatt: BluetoothGatt?,
                status: Int,
                newState: Int
            ) {
                super.onConnectionStateChange(gatt, status, newState)
                if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
                    return
                }
                if (newState == BluetoothProfile.STATE_CONNECTED) {
                    bluetoothGatt = gatt
                    outputText("Connected")
                    // this was the reason android couldn't connect to macOS? no, was the setLegacy(false). diagnosed by comparing nRF Connect logs from Flying Carpet pairings to nRF Connect pairings.
                    Thread.sleep(1600)
                    gatt?.discoverServices()
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
                Log.i("Bluetooth", "Not bonded")
                return
            }
            // outputText("Device: $peerDevice")

            if (result == null) {
                Log.e("Bluetooth", "Received ACTION_BOND_STATE_CHANGED but do not have device result")
                return
            }
            if (!bonded) {
                bonded = true
                result!!.device.connectGatt(
                    application.applicationContext,
                    true,
                    gattCallback,
                    BluetoothDevice.TRANSPORT_AUTO,
                )
            } else {
                Log.e("Bluetooth", "Received ACTION_BOND_STATE_CHANGED but already bonded")
            }
        }

        // use to read peripheral's characteristic
        fun read(characteristicUuid: UUID) {
            // outputText("Reading $characteristicUuid")
            if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
                outputText("No permission")
                return
            }
            when (characteristicUuid) {
                OS_CHARACTERISTIC_UUID -> bluetoothGatt?.readCharacteristic(osCharacteristic)
                SSID_CHARACTERISTIC_UUID -> bluetoothGatt?.readCharacteristic(ssidCharacteristic)
                PASSWORD_CHARACTERISTIC_UUID -> bluetoothGatt?.readCharacteristic(passwordCharacteristic)
            }
        }

        // private fun writeSinglePacket(characteristicUuid: UUID, value: ByteArray, waitForResponse: Boolean) {
        fun write(characteristicUuid: UUID, value: ByteArray) {
            // outputText("Writing to $characteristicUuid")
            // val writeType = if (waitForResponse) BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT else BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
            val writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
            if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
                return
            }
            val characteristic = when (characteristicUuid) {
                OS_CHARACTERISTIC_UUID -> osCharacteristic
                SSID_CHARACTERISTIC_UUID -> ssidCharacteristic
                PASSWORD_CHARACTERISTIC_UUID -> passwordCharacteristic
                else -> {
                    outputText("Bad characteristic: $characteristicUuid")
                    return
                }
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                bluetoothGatt?.writeCharacteristic(
                    characteristic!!,
                    value,
                    writeType
                )
            } else {
                characteristic?.value = value
                characteristic?.writeType = writeType
                bluetoothGatt?.writeCharacteristic(characteristic)
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

