package dev.spiegl.flyingcarpet

import android.Manifest
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
import android.graphics.Bitmap
import android.os.Build
import android.os.ParcelUuid
import android.util.Log
import androidx.core.app.ActivityCompat
import androidx.lifecycle.LiveData
import androidx.lifecycle.MutableLiveData
import java.util.UUID


val SERVICE_UUID: UUID = UUID.fromString("A70BF3CA-F708-4314-8A0E-5E37C259BE5C")
val OS_CHARACTERISTIC_UUID: UUID = UUID.fromString("BEE14848-CC55-4FDE-8E9D-2E0F9EC45946")
val WIFI_CHARACTERISTIC_UUID: UUID = UUID.fromString("0D820768-A329-4ED4-8F53-BDF364EDAC75")
class Bluetooth(
    val application: Application,
    val gotPeer: (ByteArray) -> Unit,
    val getWifiInfo: () -> String,
    val connectToPeer: () -> Unit,
) {

    lateinit var bluetoothManager: BluetoothManager
    lateinit var bluetoothGattServer: BluetoothGattServer
    private lateinit var service: BluetoothGattService
    lateinit var bluetoothLeScanner: BluetoothLeScanner
    var bluetoothReceiver = BluetoothReceiver(application, null)
    var active = false


    private var _status = MutableLiveData<Boolean>()
    val status: LiveData<Boolean>
        get() = _status

    // peripheral

    fun initializePeripheral(application: Context) {
        if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
            return
        }
        bluetoothGattServer = bluetoothManager.openGattServer(application, serverCallback)
        service = BluetoothGattService(SERVICE_UUID, BluetoothGattService.SERVICE_TYPE_PRIMARY)
        val wifiCharacteristic = BluetoothGattCharacteristic(
            WIFI_CHARACTERISTIC_UUID,
            BluetoothGattCharacteristic.PROPERTY_READ or BluetoothGattCharacteristic.PROPERTY_WRITE, // TODO: correct?
            BluetoothGattCharacteristic.PERMISSION_READ_ENCRYPTED_MITM or BluetoothGattCharacteristic.PERMISSION_WRITE_ENCRYPTED_MITM,
        )
        val osCharacteristic = BluetoothGattCharacteristic(
            OS_CHARACTERISTIC_UUID,
            BluetoothGattCharacteristic.PROPERTY_READ or BluetoothGattCharacteristic.PROPERTY_WRITE, // TODO: correct?
            BluetoothGattCharacteristic.PERMISSION_READ_ENCRYPTED_MITM or BluetoothGattCharacteristic.PERMISSION_WRITE_ENCRYPTED_MITM,
        )
        service.addCharacteristic(wifiCharacteristic)
        service.addCharacteristic(osCharacteristic)
        bluetoothGattServer.addService(service)
    }

    private val serverCallback = object : BluetoothGattServerCallback() {
        override fun onConnectionStateChange(device: BluetoothDevice?, status: Int, newState: Int) {
            super.onConnectionStateChange(device, status, newState)
            if (newState == BluetoothProfile.STATE_CONNECTED) {
                Log.i("Bluetooth", "Device connected")
            } else {
                Log.i("Bluetooth", "Device disconnected")
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
            when (characteristic.uuid) { // TODO
                // tell peer we're android
                OS_CHARACTERISTIC_UUID -> {
                    bluetoothGattServer.sendResponse(
                        device, requestId, BluetoothGatt.GATT_SUCCESS, 0, "android".toByteArray()
                    )
                }
                // must have started wifi hotspot by this point, so send ssid and password
                WIFI_CHARACTERISTIC_UUID -> {
                    bluetoothGattServer.sendResponse(
                        device, requestId, BluetoothGatt.GATT_SUCCESS, 0, "$getWifiInfo()".toByteArray()
                    )
                }
                else -> {
                    Log.i("Bluetooth", "Invalid characteristic")
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
            if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
                return
            }
            if (characteristic == null) {
                return
            }
            when (characteristic.uuid) {
                OS_CHARACTERISTIC_UUID -> {
                    // now we know peer's OS, so figure out hosting and connect
                    value?.let { gotPeer(it) }
                }
                WIFI_CHARACTERISTIC_UUID -> {
                    // TODO:
                    //    if peer is writing wifi details to us, we're joining. but we already know that because OS characteristic is already written, so we can just call connectToPeer?
                    //    no, connectToPeer assumes no bluetooth because it launches QR scanner? - fixed
                    //    what do we actually need to do? just join. but really we shouldn't be scanning qr code in connectToPeer unless we're not using bluetooth?
                    //    and shouldn't be showing QR code unless we're not using bluetooth, but have to take care of that in localOnlyHotspotCallback.onStarted callback where we get the wifi details.
                    //    if using bluetooth, connectToPeer won't need to scan QR code because it will already have wifi details.
                    //    can call connectToPeer() or joinHotspot here()?
                    connectToPeer()
                }
                else -> {
                    Log.i("Bluetooth", "Invalid characteristic")
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
            .setIncludeDeviceName(true)
            .setIncludeTxPowerLevel(false)
            .addServiceUuid(ParcelUuid(SERVICE_UUID))
            .build()
        bluetoothLeAdvertiser.startAdvertising(settings, data, advertiseCallback)
    }

    private val advertiseCallback = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings?) {
            super.onStartSuccess(settingsInEffect)
            _status.postValue(true)
            Log.i("Bluetooth", "Advertiser started")
        }

        override fun onStartFailure(errorCode: Int) {
            super.onStartFailure(errorCode)
            Log.i("Bluetooth", "Advertiser failed to start: $errorCode")
        }
    }

    // central

    fun initializeCentral() {
        // TODO: nothing to do in this function and this should all go to scan()?
        //    bluetoothManager will have an adapter, and
    }

    fun scan() {
        if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_SCAN) != PackageManager.PERMISSION_GRANTED) {
            return
        }
        val scanFilter = ScanFilter.Builder()
            .setServiceUuid(ParcelUuid(SERVICE_UUID))
            .build()
        val scanSettings = ScanSettings.Builder()
            .setLegacy(false)
            .build()
        bluetoothManager.adapter.bluetoothLeScanner.startScan(listOf(scanFilter), scanSettings, leScanCallback)
        // bluetoothLeScanner = bluetoothManager.adapter.bluetoothLeScanner
        // bluetoothLeScanner.startScan(listOf(scanFilter), scanSettings, leScanCallback)
        // bluetoothLeScanner.startScan(leScanCallback)
    }

    // TODO: use read and write when receiving... we call scan, scan bonds, then BluetoothReceiver reads OS and kicks us off?
    fun read(characteristicUuid: UUID) {
        if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
            return
        }
        when (characteristicUuid) {
            OS_CHARACTERISTIC_UUID -> bluetoothReceiver.bluetoothGatt?.readCharacteristic(bluetoothReceiver.osCharacteristic)
            WIFI_CHARACTERISTIC_UUID -> bluetoothReceiver.bluetoothGatt?.readCharacteristic(bluetoothReceiver.wifiCharacteristic)
        }
    }

    fun write(characteristicUuid: UUID, value: ByteArray) {
        if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
            return
        }
        when (characteristicUuid) {
            OS_CHARACTERISTIC_UUID -> {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    bluetoothReceiver.bluetoothGatt?.writeCharacteristic(
                        bluetoothReceiver.osCharacteristic!!,
                        value,
                        BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
                    )
                } else {
                    // this takes place in the context of being a central. the peerDevice will have discoverable characteristics.
                    // we should've discovered them by this point?
                    bluetoothReceiver.osCharacteristic?.value = value
                    bluetoothReceiver.bluetoothGatt?.writeCharacteristic(bluetoothReceiver.osCharacteristic)
                }
            }
            WIFI_CHARACTERISTIC_UUID -> {
                bluetoothReceiver.wifiCharacteristic?.value = value
                bluetoothReceiver.bluetoothGatt?.writeCharacteristic(bluetoothReceiver.wifiCharacteristic)
            }
        }
    }

    private val leScanCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult?) {
            super.onScanResult(callbackType, result)
            if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_SCAN) != PackageManager.PERMISSION_GRANTED) {
                return
            }
            Log.i("Bluetooth", "Scan result: $result")
            if (result != null) {
//                address = result.device.address
                _status.postValue(true)
                bluetoothReceiver.result = result
                result.device.createBond()
                bluetoothLeScanner.stopScan(this)
            }
        }

        override fun onScanFailed(errorCode: Int) {
            Log.e("Bluetooth", "Scan failed: $errorCode")
            super.onScanFailed(errorCode)
        }
    }

    // this class receives the bluetooth bonded events
    class BluetoothReceiver(private val application: Application, var result: ScanResult?): BroadcastReceiver() {

        var peerDevice: BluetoothDevice? = null
        var bluetoothGatt: BluetoothGatt? = null
        var osCharacteristic: BluetoothGattCharacteristic? = null
        var wifiCharacteristic: BluetoothGattCharacteristic? = null
//        private var _receivedData = MutableLiveData<String>()
//        val receivedData: LiveData<String>
//            get() = _receivedData

        override fun onReceive(context: Context?, intent: Intent?) {
            Log.i("Bluetooth", "Action: ${intent?.action}")
            peerDevice = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                intent?.getParcelableExtra(EXTRA_DEVICE, BluetoothDevice::class.java)
            } else {
                intent?.getParcelableExtra(EXTRA_DEVICE)
            }
//            if (device?.address != address) {
//                return
//            }
            val bondState = intent?.getIntExtra(EXTRA_BOND_STATE, -1)
            if (bondState != BOND_BONDED) {
                Log.i("Bluetooth", "Not bonded")
                return
            }
            Log.i("Bluetooth", "Device: $peerDevice")

            val gattCallback = object : BluetoothGattCallback() {
                override fun onCharacteristicRead(
                    gatt: BluetoothGatt,
                    characteristic: BluetoothGattCharacteristic,
                    value: ByteArray,
                    status: Int
                ) {
                    super.onCharacteristicRead(gatt, characteristic, value, status)
                    val stringRepresentation = value.toString(Charsets.UTF_8)
                    Log.i("Bluetooth", "Read characteristic: $stringRepresentation")
                    // TODO: we're central, so receiving. if we read OS characteristic, we know whether to start hotspot or join it.
                    //    if we start it, we need to write the details. if we're joining, need to read them.
                    //    use liveData? no. pass a bunch of callbacks into here?
//                    _receivedData.postValue(stringRepresentation)
                    osReadCallback
                    when (characteristic.uuid) {
                        OS_CHARACTERISTIC_UUID -> {
                            if (isHosting()) {

                            }
                        }
                        WIFI_CHARACTERISTIC_UUID -> {

                        }
                    }
                }

                override fun onServicesDiscovered(gatt: BluetoothGatt?, status: Int) {
                    if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
                        return
                    }
                    super.onServicesDiscovered(gatt, status)
                    Log.i("Bluetooth", "Discovered services")
                    for (service in gatt?.services!!) {
                        Log.i("Bluetooth", "Service: ${service.uuid}")
                    }
                    val service = gatt.getService(SERVICE_UUID) ?: return
                    Log.i("Bluetooth", "Got service: $service")
                    osCharacteristic = service.getCharacteristic(OS_CHARACTERISTIC_UUID) ?: return
                    wifiCharacteristic = service.getCharacteristic(WIFI_CHARACTERISTIC_UUID) ?: return
                    Log.i("Bluetooth", "Got characteristics: $osCharacteristic, $wifiCharacteristic")

                }

                override fun onServiceChanged(gatt: BluetoothGatt) {
                    super.onServiceChanged(gatt)
                    if (ActivityCompat.checkSelfPermission(application, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
                        return
                    }
                    Log.i("Bluetooth", "Services changed")
                    gatt.discoverServices()
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
                    bluetoothGatt = gatt
                    Log.i("Bluetooth", "Connected")
                    gatt?.discoverServices()
                }
            }
            if (result == null) {
                Log.e("Bluetooth", "Received ACTION_BOND_STATE_CHANGED but do not have device result")
                return
            }
            result!!.device.connectGatt(application.applicationContext, false, gattCallback)
        }

    }

}

