package dev.spiegl.flyingcarpet

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
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.BluetoothLeAdvertiser
import android.bluetooth.le.BluetoothLeScanner
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanResult
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import android.util.Log
import java.util.UUID


val SERVICE_UUID: UUID = UUID.fromString("A70BF3CA-F708-4314-8A0E-5E37C259BE5C")
val OS_CHARACTERISTIC_UUID: UUID = UUID.fromString("BEE14848-CC55-4FDE-8E9D-2E0F9EC45946")
val WIFI_CHARACTERISTIC_UUID: UUID = UUID.fromString("0D820768-A329-4ED4-8F53-BDF364EDAC75")
class Bluetooth(application: Application) {

    lateinit var bluetoothManager: BluetoothManager
    lateinit var bluetoothGattServer: BluetoothGattServer
    lateinit var bluetoothLeAdvertiser: BluetoothLeAdvertiser
    lateinit var service: BluetoothGattService
    lateinit var bluetoothLeScanner: BluetoothLeScanner
    var bluetoothReceiver = BluetoothReceiver(application, null)
//    lateinit var address: String // address of device advertising service that we as central are trying to connect to

    // peripheral
    val serverCallback = object : BluetoothGattServerCallback() {
        override fun onConnectionStateChange(device: BluetoothDevice?, status: Int, newState: Int) {
            super.onConnectionStateChange(device, status, newState)
            if (newState == BluetoothProfile.STATE_CONNECTED) {
                Log.i("Bluetooth", "Device connected")
            } else {
                Log.i("Bluetooth", "Device disconnected")
            }
        }

        @SuppressLint("MissingPermission")
        override fun onCharacteristicReadRequest(
            device: BluetoothDevice?,
            requestId: Int,
            offset: Int,
            characteristic: BluetoothGattCharacteristic?
        ) {
            super.onCharacteristicReadRequest(device, requestId, offset, characteristic)

            if (characteristic == null || characteristic.uuid != CHARACTERISTIC_UUID) {
                Log.i("Bluetooth", "Invalid characteristic")
                bluetoothGattServer.sendResponse(device, requestId, BluetoothGatt.GATT_REQUEST_NOT_SUPPORTED, 0, null)
                return
            }
            bluetoothGattServer.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, "wifi:password".toByteArray())
        }
    }

    val advertiseCallback = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings?) {
            super.onStartSuccess(settingsInEffect)
            // TODO: turn icon blue
            Log.i("Bluetooth", "Advertiser started")
        }

        override fun onStartFailure(errorCode: Int) {
            super.onStartFailure(errorCode)
            Log.i("Bluetooth", "Advertiser failed to start: $errorCode")
        }
    }

    // central
    val leScanCallback = object : ScanCallback() {
        @SuppressLint("MissingPermission")
        override fun onScanResult(callbackType: Int, result: ScanResult?) {
            super.onScanResult(callbackType, result)
            Log.i("Bluetooth", "Scan result: $result")
            if (result != null) {
//                address = result.device.address
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

    class BluetoothReceiver(private val application: Application, var result: ScanResult?): BroadcastReceiver() {

        @SuppressLint("MissingPermission")
        override fun onReceive(context: Context?, intent: Intent?) {
            Log.i("Bluetooth", "Action: ${intent?.action}")
            val device: BluetoothDevice? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
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
            Log.i("Bluetooth", "Device: $device")

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
                }

                override fun onServicesDiscovered(gatt: BluetoothGatt?, status: Int) {
                    super.onServicesDiscovered(gatt, status)
                    Log.i("Bluetooth", "Discovered services")
                    for (service in gatt?.services!!) {
                        Log.i("Bluetooth", "Service: ${service.uuid}")
                    }
                    val service = gatt.getService(SERVICE_UUID) ?: return
                    Log.i("Bluetooth", "Got service: $service")
                    val characteristic = service.getCharacteristic(CHARACTERISTIC_UUID) ?: return
                    Log.i("Bluetooth", "Got characteristic: $characteristic")
                    gatt.readCharacteristic(characteristic)
                }

                override fun onServiceChanged(gatt: BluetoothGatt) {
                    super.onServiceChanged(gatt)
                    Log.i("Bluetooth", "Services changed")
                    gatt.discoverServices()
                }

                override fun onConnectionStateChange(
                    gatt: BluetoothGatt?,
                    status: Int,
                    newState: Int
                ) {
                    super.onConnectionStateChange(gatt, status, newState)
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

