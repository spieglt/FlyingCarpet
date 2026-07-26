//
//  Bluetooth.swift
//  FlyingCarpet
//
//  Created by Theron on 6/12/24.
//

import CoreBluetooth
import Foundation

// problem is that when windows central pairs to ios peripheral, it doesn't find the flying carpet service.
// could it be that ios doesn't add the service to the peripheral manager or whatever till after pair is pressed?
// we don't get notified when pair is pressed? but still maybe possible to add the service earlier?
// problem was we were adding service to peripheralmanager in central's state update. led to inconsistent bug where
// error would be thrown if central tried to add it to peripheralmanager before peripheralmanager was poweredOn.

class Bluetooth: NSObject {

    var peripheralManager: CBPeripheralManager? = nil
    var centralManager: CBCentralManager? = nil
    var discoveredPeripheral: CBPeripheral? = nil
    static let serviceUuid = CBUUID(string: "A70BF3CA-F708-4314-8A0E-5E37C259BE5C")
    let osCharacteristicUuid = CBUUID(string: "BEE14848-CC55-4FDE-8E9D-2E0F9EC45946")
    let ssidCharacteristicUuid = CBUUID(string: "0D820768-A329-4ED4-8F53-BDF364EDAC75")
    let passwordCharacteristicUuid = CBUUID(string: "E1FA8F66-CF88-4572-9527-D5125A2E0762")
    var service: CBMutableService? = nil

    // central characteristics
    var osCharacteristic: CBCharacteristic? = nil
    var ssidCharacteristic: CBCharacteristic? = nil
    var passwordCharacteristic: CBCharacteristic? = nil

    // peripheral characteristics
    var osMutableCharacteristic: CBMutableCharacteristic? = nil
    var ssidMutableCharacteristic: CBMutableCharacteristic? = nil
    var passwordMutableCharacteristic: CBMutableCharacteristic? = nil

    let noSsid = "NONE"
    // atomic: set from the main thread (UI) and CoreBluetooth's delegate queue
    let active = AtomicBool(false) // tracks whether bluetooth is being used, should reflect state of switch
    let capable = AtomicBool(false) // tracks whether bluetooth can be used, should not reflect state of switch
    var localPassword: String? = nil  // Password to advertise via Bluetooth (set by ViewController)

    func initialize(delegate: ViewController, queue: dispatch_queue_t?) {
        // initialize bluetooth peripheral
        peripheralManager = CBPeripheralManager.init(delegate: delegate, queue: queue)
        
        // initialize bluetooth central
        centralManager = CBCentralManager.init(delegate: delegate, queue: queue)
    }

    func stateUpdate(state: CBManagerState) -> String {
        print("state: \(state)")
        var msg: String
        switch (state) {
            case .unknown:
                msg = "Bluetooth state unknown"
                break
            case .resetting:
                msg = "Bluetooth resetting"
                break
            case .unsupported:
                msg = "Bluetooth state unsupported"
                break
            case .unauthorized:
                msg = "Bluetooth unauthorized"
                break
            case .poweredOff:
                msg = "Bluetooth powered off"
                break
            case .poweredOn:
                msg = "Bluetooth powered on"
                break
            @unknown default:
                fatalError("Uh oh")
        }
        return msg
    }

    // peripheral

    func peripheralDidUpdateState(peripheral: CBPeripheralManager) -> String {
        print("peripheral: \(peripheral)")
        addServiceIfNeeded()
        return stateUpdate(state: peripheral.state)
    }

    // Build and register the GATT service if the peripheral is powered on and it isn't
    // already registered. Extracted so it runs both on power-on and at the start of each
    // transfer (in startAdvertising), since removeService() tears it down between transfers.
    func addServiceIfNeeded() {
        guard let peripheralManager = peripheralManager,
              peripheralManager.state == .poweredOn,
              service == nil else {
            print("peripheral not ready or service already added")
            return
        }
        let newService = CBMutableService(type: Bluetooth.serviceUuid, primary: true)
        osMutableCharacteristic = CBMutableCharacteristic(
            type: osCharacteristicUuid,
            properties: [.read, .write, .notifyEncryptionRequired], // are these correct?
            value: nil,
            permissions: [.readEncryptionRequired, .writeEncryptionRequired]
        )
        ssidMutableCharacteristic = CBMutableCharacteristic(
            type: ssidCharacteristicUuid,
            properties: [.read, .write, .notifyEncryptionRequired],
            value: nil,
            permissions: [.readEncryptionRequired, .writeEncryptionRequired]
        )
        passwordMutableCharacteristic = CBMutableCharacteristic(
            type: passwordCharacteristicUuid,
            properties: [.read, .write, .notifyEncryptionRequired],
            value: nil,
            permissions: [.readEncryptionRequired, .writeEncryptionRequired]
        )
        newService.characteristics = [
            osMutableCharacteristic!,
            ssidMutableCharacteristic!,
            passwordMutableCharacteristic!,
        ]
        peripheralManager.add(newService)
        service = newService
        print("added characteristics")
    }

    // Remove our GATT service so it isn't left offering characteristics after a transfer.
    // stopAdvertising() (called first in stopBluetooth) already makes us undiscoverable;
    // this removes the registration too. addServiceIfNeeded() re-adds it on the next
    // transfer. removeAllServices() must run after advertising has stopped.
    func removeService() {
        peripheralManager?.removeAllServices()
        service = nil
        osMutableCharacteristic = nil
        ssidMutableCharacteristic = nil
        passwordMutableCharacteristic = nil
    }

    func didReceiveRead(_ peripheral: CBPeripheralManager, _ request: CBATTRequest) {
//        if peripheralManager!.isAdvertising {
//            peripheralManager!.stopAdvertising()
//            print("Stopped advertising")
//        }
        print("in read handler: \(request.characteristic.uuid)")
        switch request.characteristic.uuid {
        case osCharacteristicUuid:
            #if os(iOS)
            request.value = Data("ios".utf8)
            #elseif os(macOS)
            print("received characteristic read, writing mac")
            request.value = Data("mac".utf8)
            #endif
            break
        case ssidCharacteristicUuid:
            // In shared network mode, no SSID needed
            request.value = Data(noSsid.utf8)
            break
        case passwordCharacteristicUuid:
            if let password = localPassword {
                print("writing password")
                request.value = Data(password.utf8)
            } else {
                print("no password set, writing empty")
                request.value = Data("".utf8)
            }
            break
        default:
            print("read of \(request.characteristic.uuid) not permitted")
            peripheral.respond(to: request, withResult: .readNotPermitted)
            return
        }
        peripheral.respond(to: request, withResult: .success)
    }

    func startAdvertising() {
        // re-register the service in case a previous transfer removed it in removeService()
        addServiceIfNeeded()
        if !peripheralManager!.isAdvertising {
            peripheralManager!.startAdvertising([
                CBAdvertisementDataLocalNameKey: "FlyingCarpet",
                CBAdvertisementDataServiceUUIDsKey: [Bluetooth.serviceUuid]
            ])
            print("started advertising")
        } else {
            print("was already advertising")
        }
    }


    // central

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        print("connected to: \(peripheral)")
        peripheral.discoverServices([Bluetooth.serviceUuid])
    }

    func centralManagerDidUpdateState(_ central: CBCentralManager) -> String {
        return stateUpdate(state: central.state)
    }

    func scan() {
        if centralManager != nil {
            let connectedPeripherals = centralManager!.retrieveConnectedPeripherals(withServices: [Bluetooth.serviceUuid])
            if connectedPeripherals.count > 0 {
                print("already connected at the system level")
                // This path skips didDiscover, which is where the scan path stores the
                // peripheral and assigns its delegate — without doing both here too, the
                // CBPeripheral could be deallocated (silently canceling the connect), the
                // discovery callbacks after didConnect went to a nil delegate, and
                // read()/write() no-op'd on a nil discoveredPeripheral. The central
                // manager's delegate (the ViewController) is also the CBPeripheralDelegate
                // on both targets.
                discoveredPeripheral = connectedPeripherals[0]
                discoveredPeripheral!.delegate = centralManager!.delegate as? CBPeripheralDelegate
                centralManager!.connect(discoveredPeripheral!)
            } else {
                centralManager!.scanForPeripherals(withServices: [Bluetooth.serviceUuid])
            }
        }
    }
    
    func didDiscoverPeripheral(
        didDiscover peripheral: CBPeripheral,
        advertisementData: [String : Any],
        rssi RSSI: NSNumber
    ) {
        print("discovered: \(peripheral)")
        // Logging only, so never crash on it: the key can be absent when the UUID arrives
        // in the overflow area (e.g. a backgrounded iOS peripheral).
        if let services = advertisementData[CBAdvertisementDataServiceUUIDsKey] as? [CBUUID] {
            for service in services {
                print("service: \(service)")
            }
        }
        // we should be able to stop scan here because we're only scanning for our service.
        // connect to the passed peripheral (callers set discoveredPeripheral to it first,
        // which is what keeps it retained).
        centralManager?.connect(peripheral)
        centralManager?.stopScan()
    }

    // The peer's GATT database changed and CoreBluetooth has invalidated our cached
    // CBService objects for it. Every Flying Carpet peripheral removes its service when a
    // transfer ends and re-adds it on the next one, so for a bonded peer this fires exactly
    // when the service we care about comes back. Without re-discovering, the cached handles
    // stay stale and the next characteristic read fails.
    //
    // Both targets call this; the delegate method itself has to live on each ViewController
    // because they are the CBPeripheralDelegate. macOS previously did not implement it at
    // all, and iOS only logged. See docs/bluetooth-field-guide.md in the FlyingCarpet repo.
    func didModifyServices(peripheral: CBPeripheral, invalidatedServices: [CBService]) {
        print("invalidatedServices: \(invalidatedServices)")
        guard invalidatedServices.contains(where: { $0.uuid == Bluetooth.serviceUuid }) else {
            return
        }
        print("our service was invalidated, re-discovering")
        peripheral.discoverServices([Bluetooth.serviceUuid])
    }

    func didDiscoverServices(peripheral: CBPeripheral, didDiscoverServices error: (any Error)?) {
        guard let services = peripheral.services else {
            print("Could not discover services: \(String(describing: error))")
            return
        }
        for service in services {
            print("Discovered service: \(service.uuid)")
            if service.uuid == Bluetooth.serviceUuid {
                peripheral.discoverCharacteristics([
                    osCharacteristicUuid,
                    ssidCharacteristicUuid,
                    passwordCharacteristicUuid,
                ], for: service)
            }
        }
    }

    func didDiscoverCharacteristics(peripheral: CBPeripheral, service: CBService, error: (any Error)?) -> String {
        guard let characteristics = service.characteristics else {
            return "No characteristics"
        }
        var msg = ""
        for characteristic in characteristics {
            switch characteristic.uuid {
            case osCharacteristicUuid:
                osCharacteristic = characteristic
            case ssidCharacteristicUuid:
                ssidCharacteristic = characteristic
            case passwordCharacteristicUuid:
                passwordCharacteristic = characteristic
            default:
                break
            }
            if msg != "" {
                msg += "\n"
            }
            msg += "Characteristic: \(characteristic.uuid)"
            // Only Apple peripherals declare notify on these characteristics; the
            // Windows/Linux/Android ones are read/write only, and subscribing to a
            // characteristic that can't notify just produces an error callback.
            if characteristic.properties.contains(.notify) || characteristic.properties.contains(.indicate) {
                peripheral.setNotifyValue(true, for: characteristic)
            }
        }
        // kick off reading peer's OS
        if osCharacteristic != nil {
            read(characteristic: osCharacteristic!)
        } else {
            return "Did not discover OS characteristic"
        }

        return msg
    }
    
    func didWriteValue(peripheral: CBPeripheral, characteristic: CBCharacteristic, error: (any Error)?) {
        switch characteristic.uuid {
        case osCharacteristic?.uuid:
            // in android, as central, after writing OS to peer, we'd connectToPeer() and start or join hotspot.
            // if joining, we'd first read peer's SSID characteristic. in this codebase, we don't have a separate
            // connectToPeer() function because we always join, so can just read SSID here.
            if ssidCharacteristic != nil {
                read(characteristic: ssidCharacteristic!)
            }
            break
        case ssidCharacteristic?.uuid:
            break
        case passwordCharacteristic?.uuid:
            break
        default:
            break
        }
    }
    
    func read(characteristic: CBCharacteristic) {
        discoveredPeripheral?.readValue(for: characteristic)
    }
    
    func write(message: String, characteristic: CBCharacteristic) {
        discoveredPeripheral?.writeValue(Data(message.utf8), for: characteristic, type: .withResponse)
    }

    // Tear down the central-side connection to the peer's peripheral. stopScan() only stops
    // discovery; any GATT connection we opened as central (when this device is receiving)
    // stays alive after the transfer until Bluetooth resets. cancelPeripheralConnection
    // requires us to still hold the CBPeripheral, which discoveredPeripheral does.
    func disconnectPeripheral() {
        if let peripheral = discoveredPeripheral {
            centralManager?.cancelPeripheralConnection(peripheral)
        }
        discoveredPeripheral = nil
    }
}
