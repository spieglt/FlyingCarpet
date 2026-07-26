//
//  ViewController.swift
//  FlyingCarpet
//
//  Created by Theron on 6/9/24.
//

import Cocoa
import CoreBluetooth
import CoreImage
import CoreLocation
@preconcurrency import CoreWLAN
import SecurityFoundation

let auth: SFAuthorization? = nil

class ViewController: NSViewController, CBPeripheralManagerDelegate, CBCentralManagerDelegate, CBPeripheralDelegate, Transfer.Delegate, NSTextFieldDelegate {

    var transfer = Transfer()
    let bluetooth = Bluetooth()
    let queue = DispatchQueue(label: "bluetooth")
    // created with the view controller so it lives on the main thread, which has the
    // run loop CLLocationManager needs; used to trigger the location permission that
    // CoreWLAN SSID scans require
    let locationManager = CLLocationManager()
    var bluetoothIsInitialized = false
    // set when we detect the BT peer is another Apple device in hotspot mode; suppresses
    // the rest of the BT exchange while we notify the peer and tear down (see didWriteValueFor)
    var blockingAppleToApple = false
    @IBOutlet weak var bluetoothSwitch: NSSwitch!
    @IBOutlet var outputField: NSTextView!
    @IBOutlet weak var progressBar: NSProgressIndicator!
    @IBOutlet weak var modeSwitch: NSSegmentedControl!
    @IBOutlet weak var connectionModeSwitch: NSSegmentedControl!  // Hotspot / Shared Network
    @IBOutlet weak var peerLabel: NSTextField!
    @IBOutlet weak var peerSwitch: NSSegmentedControl!
    @IBOutlet weak var startTransferButton: NSButton!
    @IBOutlet weak var cancelButton: NSButton!
    @IBOutlet weak var bluetoothIcon: NSImageView!
    @IBOutlet weak var logoImageView: NSImageView!  // shows the QR code while receiving in shared network mode
    @IBOutlet weak var peerBox: NSStackView!
    //@IBOutlet weak var peerBoxConstraint: NSLayoutConstraint!

    override func viewDidLoad() {
        super.viewDidLoad()
        transfer.delegate = self
        bluetooth.initialize(delegate: self, queue: queue)
        self.output(msg: "Bluetooth initialized")
    }

    override var representedObject: Any? {
        didSet {
            // Update the view, if already loaded.
        }
    }

    func output(msg: String) {
        DispatchQueue.main.async {
            self.outputField.textStorage?.mutableString.setString(
                (self.outputField.textStorage?.string ?? "") + "\n" + msg
            )
            self.outputField.scrollToEndOfDocument(nil)
        }
    }

    func setProgress(_ progress: Float, animated: Bool, hidden: Bool) {
        DispatchQueue.main.async {
            if self.progressBar.isHidden && !hidden {
                self.progressBar.isHidden = false
            } else if !self.progressBar.isHidden && hidden {
                self.progressBar.isHidden = true
            }
            self.progressBar.doubleValue = Double(progress) * 100
        }
    }

    @IBAction func peerSwitchOnClick(_ sender: NSSegmentedControl) {
        // no-op: the SSID (Android only) is entered in a prompt after file selection,
        // not a persistent box tied to the peer selection
    }
    
    @IBAction func modeSwitchOnClick(_ sender: NSSegmentedControl) {
        if self.modeSwitch.selectedSegment == 0 {
            self.startTransferButton.title = "Select Files"
        } else {
            self.startTransferButton.title = "Select Folder"
        }
    }

    @IBAction func connectionModeSwitched(_ sender: NSSegmentedControl) {
        if sender.selectedSegment == 0 {
            transfer.connectionMode = .hotspot
            bluetoothSwitch.isEnabled = bluetoothSwitchShouldBeEnabled()
            // restore the hotspot-mode fields hidden by shared network mode
            bluetoothSwitchFlipped(bluetoothSwitch)
        } else {
            transfer.connectionMode = .sharedNetwork
            // Bluetooth isn't used in shared network mode; turn it off and disable the switch.
            bluetoothSwitch.state = .off
            bluetoothSwitchFlipped(bluetoothSwitch)
            bluetoothSwitch.isEnabled = false
            // Peer OS doesn't matter in shared network mode (discovery finds the peer
            // over IP), and the password is handled after file selection: the receiver
            // displays it and the sender is prompted to enter it.
            peerLabel.isHidden = true
            peerSwitch.isHidden = true
        }
    }

    @IBAction func aboutOnClick(_ sender: Any) {
        (NSApp.delegate as? AppDelegate)?.showAbout(sender)
    }
    
    @IBAction func startButtonOnClick(_ sender: Any) {

        // disable UI during transfer
        toggleUI(transferRunning: true)

        // reset the Apple-to-Apple abort flag for this run
        blockingAppleToApple = false

        // refresh transfer
        self.transfer = Transfer()
        transfer.delegate = self

        // Set connection mode from UI
        if connectionModeSwitch.selectedSegment == 0 {
            transfer.connectionMode = .hotspot
        } else {
            transfer.connectionMode = .sharedNetwork
        }

        // Bluetooth is only used in hotspot mode. In shared network mode the password
        // is exchanged manually (receiver displays it, sender types it).
        let useBluetooth = bluetooth.active.value && transfer.connectionMode != .sharedNetwork

        let panel = NSOpenPanel()
        panel.canChooseDirectories = true

        // set up file/folder selection
        switch modeSwitch.selectedSegment {
        case 0: // Send
            // show file picker
            self.transfer.mode = .Sending
            panel.allowsMultipleSelection = true
            panel.canChooseFiles = true
            break
        case 1: // Receive
            // show folder picker
            self.transfer.mode = .Receiving
            panel.allowsMultipleSelection = false
            panel.canChooseFiles = false
            break
        default:
            self.output(msg: "Must select whether this device is sending or receiving.")
            self.transfer.cleanUpTransfer()
            return
        }

        // validate the peer OS before file selection. this is a quick configuration
        // check (does the transfer make sense at all); the SSID and password, which the
        // user fetches from the other device at connection time, are collected after
        // files are chosen (in finishStartingTransfer) so file selection isn't gated on
        // them. in shared network mode the peer OS doesn't matter (discovery finds the
        // peer over IP), so there's nothing to check here.
        if !useBluetooth && transfer.connectionMode != .sharedNetwork {
            switch peerSwitch.selectedSegment {
            case 0, 1, 2: // Android, Linux, Windows
                break
            case 3, 4: // macOS, iOS
                self.output(msg: "Apple-to-Apple transfers require Shared Network mode. Please select Shared Network and ensure both devices are on the same network (one device can host a hotspot that the other has manually joined).")
                self.transfer.cleanUpTransfer()
                return
            default:
                self.output(msg: "Must choose OS of the other device.")
                self.transfer.cleanUpTransfer()
                return
            }
        }

        // show file/folder picker
        switch panel.runModal() {
        case .OK:
            // read the selection here: NSOpenPanel is UI, so panel.urls has to be touched
            // on the main thread, not inside the background block below
            let urls = panel.urls
            // enumerating a large folder can take a while; keep it off the main thread
            DispatchQueue.global().async {
                do {
                    try self.transfer.handleFileSelection(urls: urls)
                } catch {
                    self.output(msg: "Could not read contents of files chosen: \(error)")
                    self.transfer.cleanUpTransfer()
                    return
                }
                DispatchQueue.main.async {
                    self.finishStartingTransfer(useBluetooth: useBluetooth)
                }
            }
        case .cancel:
            self.output(msg: "File/folder selection cancelled, exiting.")
            self.transfer.cleanUpTransfer()
        default:
            finishStartingTransfer(useBluetooth: useBluetooth)
        }
    }

    // continuation of startButtonOnClick, after file selection has been processed
    private func finishStartingTransfer(useBluetooth: Bool) {
        // shared network mode: files are chosen, now handle the password. the receiver
        // generates and displays it; the sender types in the one shown on the receiver.
        if transfer.connectionMode == .sharedNetwork {
            if transfer.mode == .Receiving {
                let password = generatePassword()
                transfer.password = password
                self.output(msg: "Password: \(password)")
                self.output(msg: "Enter this password on the sending device, or scan the QR code.")
                // display the password as a QR code so an iOS/Android sender can scan it
                // instead of typing. The content is the bare password, matching the desktop
                // and Android shared-network receivers (hotspot QR codes are "ssid;password").
                let qr = qrCodeImage(from: password, size: 220)
                // replace the logo on the main screen with the QR code (like the other
                // platforms); reset to the logo in toggleUI(transferRunning: false).
                if let qr = qr {
                    self.logoImageView.image = qr
                }
                let alert = NSAlert()
                alert.messageText = "Flying Carpet"
                alert.informativeText = "Start the transfer on the sending device and enter this password when prompted, or scan the QR code:\n\n\(password)"
                if let qr = qr {
                    let imageView = NSImageView(frame: NSRect(x: 0, y: 0, width: 220, height: 220))
                    imageView.image = qr
                    imageView.imageScaling = .scaleProportionallyUpOrDown
                    alert.accessoryView = imageView
                }
                // shown as a sheet so it doesn't block: the transfer (started below)
                // must not wait for the alert to be dismissed
                if let window = self.view.window {
                    alert.beginSheetModal(for: window)
                } else {
                    alert.runModal()
                }
            } else {
                guard let password = promptForPassword(minLength: 10) else {
                    self.output(msg: "Transfer cancelled.")
                    self.transfer.cleanUpTransfer()
                    return
                }
                transfer.password = password
            }
        }

        // hotspot mode without Bluetooth: the sender enters the credentials shown on the
        // device hosting the hotspot, now that files are selected. Android hosts advertise
        // an OS-assigned SSID, so it must be entered alongside the password; every other
        // peer uses an SSID derived from the password, so only the password is needed.
        if !useBluetooth && transfer.connectionMode == .hotspot {
            if peerSwitch.selectedSegment == 0 { // Android
                guard let creds = promptForSsidAndPassword() else {
                    self.output(msg: "Transfer cancelled.")
                    self.transfer.cleanUpTransfer()
                    return
                }
                transfer.ssid = creds.ssid
                transfer.password = creds.password
            } else {
                guard let password = promptForPassword(minLength: 8) else {
                    self.output(msg: "Transfer cancelled.")
                    self.transfer.cleanUpTransfer()
                    return
                }
                transfer.password = password
            }
        }

        // start bluetooth advertisement if sending or scanning if receiving
        if useBluetooth {
            if self.transfer.mode == .Sending {
                // Hotspot mode: sender generates password for the hotspot
                let password = generatePassword()
                transfer.password = password
                bluetooth.localPassword = password
                bluetooth.startAdvertising()
            } else {
                self.output(msg: "Starting Bluetooth scan, waiting for sending device...")
                bluetooth.scan()
            }
        } else {
            self.transfer.task = Task {
                await self.transfer.runTransfer()
            }
        }
    }

    @IBAction func cancelButtonOnClick(_ sender: NSButton) {
        print("transfer cancelled")
        self.transfer.cleanUpTransfer()
    }

    // asks for the password displayed on the other device: the receiver's generated
    // password in shared network mode (minLength 10, matching generated passwords), or
    // the hosting device's in hotspot mode (minLength 8: an Android host's password is
    // the OS-generated WPA2 passphrase, whose length we don't control). returns nil if
    // the user cancels.
    func promptForPassword(minLength: Int) -> String? {
        var message = "Enter the password displayed on the receiving device:"
        while true {
            let alert = NSAlert()
            alert.messageText = "Flying Carpet"
            alert.informativeText = message
            alert.addButton(withTitle: "OK")
            alert.addButton(withTitle: "Cancel")
            let input = NSTextField(frame: NSRect(x: 0, y: 0, width: 240, height: 24))
            alert.accessoryView = input
            alert.window.initialFirstResponder = input
            if alert.runModal() != .alertFirstButtonReturn {
                return nil
            }
            let password = input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            if password.count >= minLength {
                return password
            }
            message = "Password must be at least \(minLength) characters. Enter the password displayed on the receiving device:"
        }
    }

    // hotspot mode with an Android peer: ask for the SSID and password the Android
    // device displays (its hotspot SSID is OS-assigned, not derivable). returns nil if
    // the user cancels.
    func promptForSsidAndPassword() -> (ssid: String, password: String)? {
        var message = "Enter the SSID and password shown on the Android device:"
        while true {
            let alert = NSAlert()
            alert.messageText = "Flying Carpet"
            alert.informativeText = message
            alert.addButton(withTitle: "OK")
            alert.addButton(withTitle: "Cancel")

            // SsidBox pre-fills "AndroidShare_" on focus (matching the old SSID field) and
            // leaves the cursor at the end, so the user types only the suffix Android shows
            let ssidField = SsidBox(frame: NSRect(x: 0, y: 30, width: 240, height: 24))
            ssidField.placeholderString = "SSID"
            let passwordField = NSSecureTextField(frame: NSRect(x: 0, y: 0, width: 240, height: 24))
            passwordField.placeholderString = "Password"
            let container = NSView(frame: NSRect(x: 0, y: 0, width: 240, height: 54))
            container.addSubview(ssidField)
            container.addSubview(passwordField)
            alert.accessoryView = container
            alert.window.initialFirstResponder = ssidField

            if alert.runModal() != .alertFirstButtonReturn {
                return nil
            }
            let ssid = ssidField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            let password = passwordField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            if ssid.isEmpty {
                message = "SSID cannot be empty. Enter the SSID and password shown on the Android device:"
                continue
            }
            if password.count < 8 {
                message = "Password must be at least 8 characters. Enter the SSID and password shown on the Android device:"
                continue
            }
            return (ssid, password)
        }
    }

    func getWiFiInterface() -> String? {
        let interfaces = CWWiFiClient.shared().interfaceNames()
        return interfaces?[0] // TODO: return multiple, select at start of transfer like other OSes?
    }

    func joinHotspot() async throws {
        guard let interface = CWWiFiClient.shared().interface() else {
            throw TransferError.NoWifiInterface
        }
        DispatchQueue.main.async {
            self.locationManager.startUpdatingLocation()
        }
//            print("SSID: \(self.transfer.ssid)")
//            let networks = try interface.scanForNetworks(withName: nil)
        // TODO: shell out to networksetup?
        // CoreWLAN's scan and associate block for several seconds each; run them on a
        // dispatch queue instead of pinning a cooperative-pool thread
        let ssid = self.transfer.ssid
        let password = self.transfer.password
        // freeze the "networks we were already on" set before association starts:
        // associate() returns before DHCP on the hotspot finishes, and until it does the
        // wifi path still advertises the old gateway
        self.transfer.latchPreJoinGateways()
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            DispatchQueue.global().async {
                do {
                    let networks = try interface.scanForNetworks(withName: ssid)
                    print("found networks: \(networks)")
                    guard let network = networks.first else {
                        throw TransferError.CouldNotFindSsid
                    }
                    try interface.associate(to: network, password: password)
                    continuation.resume()
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
        try await self.transfer.awaitPeerGateway()
    }

    func toggleUI(transferRunning: Bool) {
        if transferRunning {
            bluetoothSwitch.isEnabled = false
        } else {
            bluetoothSwitch.isEnabled = bluetoothSwitchShouldBeEnabled()
            // restore the logo, replaced by the QR code while receiving in shared network mode
            logoImageView.image = NSImage(named: "fc1024")
        }
        modeSwitch.isEnabled = !transferRunning
        connectionModeSwitch.isEnabled = !transferRunning
        peerSwitch.isEnabled = !transferRunning
        startTransferButton.isHidden = transferRunning
        cancelButton.isHidden = !transferRunning
    }

    func forgetHotspot() {
        guard let interface = getWiFiInterface() else {
            return
        }
        let ssid = transfer.ssid
        if ssid == "" {
            return
        }
        // networksetup can take a while; don't block the main thread, which is where
        // cleanUpTransfer calls this from.
        // Passed as argv, not interpolated into a shell command: the SSID for an Android
        // peer is whatever string that device advertised, so a shell string here would be
        // a command-injection vector.
        DispatchQueue.global().async {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/sbin/networksetup")
            process.arguments = ["-removepreferredwirelessnetwork", interface, ssid]
            let pipe = Pipe()
            process.standardOutput = pipe
            process.standardError = pipe
            do {
                try process.run()
                process.waitUntilExit()
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                print(String(data: data, encoding: .utf8) ?? "")
            } catch {
                print("networksetup failed: \(error)")
            }
        }
    }

    func emptyDocsDir() throws {
        // don't need because no Photos selection, or we should be able to read them? or should we do it the same way as in iOS?
    }

    // bluetooth

    /// Bluetooth is hotspot-only: shared network mode exchanges the password manually
    /// (display + type/QR), so the switch stays off and greyed out there. Every place that
    /// re-enables the switch must go through this, or the switch comes back live in shared
    /// network mode — which is what happened when a transfer finished and `toggleUI` restored
    /// the controls without consulting the connection mode.
    func bluetoothSwitchShouldBeEnabled() -> Bool {
        return bluetooth.capable.value && transfer.connectionMode == .hotspot
    }

    @IBAction func bluetoothSwitchFlipped(_ sender: NSSwitch) {
        if sender.state == .on {
            bluetooth.active.value = true
            // hide peer OS switch
            peerSwitch.isHidden = true
            // show bluetooth icon
            bluetoothIcon.isHidden = false
            peerLabel.isHidden = true
        } else {
            bluetooth.active.value = false
            // show peer OS switch
            peerSwitch.isHidden = false
            // hide bluetooth icon
            bluetoothIcon.isHidden = true
            bluetooth.peripheralManager?.stopAdvertising()
            bluetooth.centralManager?.stopScan()
            peerLabel.isHidden = false
            peerSwitch.isHidden = false
        }
    }

    func toggleBluetoothUI(state: CBManagerState) {
        DispatchQueue.main.async {
            switch state {
            case .poweredOn:
                self.bluetooth.capable.value = true
                // Don't switch Bluetooth on in shared network mode, where it isn't used:
                // turning the radio on in Control Center mid-session lands here.
                // bluetoothSwitchFlipped owns the icon and the peer OS switch, so skipping
                // it here also keeps those correct for shared network mode.
                if self.transfer.connectionMode == .hotspot {
                    self.bluetooth.active.value = true
                    self.bluetoothSwitch.state = .on
                    self.bluetoothSwitchFlipped(self.bluetoothSwitch)
                    self.bluetoothIcon.isHidden = false
                }
                self.bluetoothSwitch.isEnabled = self.bluetoothSwitchShouldBeEnabled()
                break
            // case .unsupported, .unauthorized, .poweredOff:
            default:
                self.bluetooth.active.value = false
                self.bluetooth.capable.value = false
                self.bluetoothSwitch.state = .off
                self.bluetoothSwitch.isEnabled = false
                self.bluetoothIcon.isHidden = true
                self.output(msg: "Bluetooth is off, not present on this system, or Bluetooth permissions were denied.")
                break
            }
        }
    }
    
    func stopBluetooth() {
        self.bluetooth.centralManager?.stopScan()
        self.bluetooth.disconnectPeripheral()
        self.bluetooth.peripheralManager?.stopAdvertising()
        self.bluetooth.removeService()
        DispatchQueue.main.async {
            self.bluetoothIcon.contentTintColor = NSColor.systemGray
        }
    }

    // peripheral

    func peripheralManagerDidUpdateState(_ peripheral: CBPeripheralManager) {
        print("Peripheral state: " + bluetooth.peripheralDidUpdateState(peripheral: peripheral))
        toggleBluetoothUI(state: peripheral.state)
    }

    func peripheralManagerDidStartAdvertising(_ peripheral: CBPeripheralManager, error: Error?) {
        output(msg: "Started Bluetooth advertisement, waiting for receiving device...")
        DispatchQueue.main.async {
            self.bluetoothIcon.contentTintColor = NSColor.systemBlue
        }
        if error != nil {
            output(msg: "Error: \(String(describing: error))")
        }
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, didAdd service: CBService, error: (any Error)?) {
        print("did add")
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, didReceiveRead request: CBATTRequest) {
        bluetooth.didReceiveRead(peripheral, request)
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, didReceiveWrite requests: [CBATTRequest]) {
        // output(msg: "Received from peer: \(String(describing: bluetooth.didReceiveWrite(peripheral, requests)))")
        // if we're peripheral, we're sending, and joining hotspot because apple, so we need to receive
        // a write from the central with the wifi details. after that, just join hotspot? runTransfer().
        //
        print("Received write: \(requests)")
        var message = ""
        for request in requests {
            if let v = request.value {
                message += String(decoding: v, as: UTF8.self)
            }
        }
        print("uuid: \(requests[0].characteristic.uuid)")
        switch requests[0].characteristic.uuid {
        case bluetooth.osCharacteristicUuid:
            self.output(msg: "Peer OS: \(message)")
            if message == "mac" || message == "ios" {
                self.output(msg: appleToAppleHotspotErrorMessage)
                self.transfer.cleanUpTransfer()
            }
            break
        case bluetooth.ssidCharacteristicUuid:
            self.output(msg: "SSID: \(message)")
            self.transfer.ssid = if message != bluetooth.noSsid { message } else { "" }
            break
        case bluetooth.passwordCharacteristicUuid:
            // Received password from peer (either hotspot host or shared network receiver)
            print("Password: \(message)")
            // hop to main: transfer.task is read there by cleanUpTransfer, and a
            // re-paired peer re-delivers the exchange — don't start a second
            // transfer over the same Transfer state
            DispatchQueue.main.async {
                guard self.transfer.task == nil else {
                    print("Transfer already started, ignoring duplicate password delivery")
                    return
                }
                self.transfer.password = message
                self.transfer.task = Task {
                    await self.transfer.runTransfer()
                }
            }
            break
        default:
            peripheral.respond(to: requests[0], withResult: .readNotPermitted)
            return
        }
        peripheral.respond(to: requests[0], withResult: .success)
    }

    // central

    func peripheral(
        _ peripheral: CBPeripheral,
        didUpdateValueFor characteristic: CBCharacteristic,
        error: (any Error)?
    ) {
        if let error = error {
            // A failed read would otherwise stall the transfer: nothing retries it. The
            // common cause is declining the system pairing prompt, which surfaces as an
            // ATT authentication/encryption error.
            let nsError = error as NSError
            let pairingCodes = [
                CBATTError.insufficientAuthentication.rawValue,
                CBATTError.insufficientEncryption.rawValue,
                CBATTError.insufficientAuthorization.rawValue,
            ]
            if nsError.domain == CBATTErrorDomain && pairingCodes.contains(nsError.code) {
                self.output(msg: "Bluetooth pairing failed or was declined. Start the transfer again to retry.")
            } else {
                self.output(msg: "Failed to read Bluetooth characteristic: \(error.localizedDescription)")
            }
            self.transfer.cleanUpTransfer()
            return
        }
        if characteristic.value == nil {
            self.output(msg: "Read Bluetooth characteristic \(characteristic.uuid) but value was nil, canceling transfer.")
            self.transfer.cleanUpTransfer()
            return
        }
        let message = String(decoding: characteristic.value!, as: UTF8.self)
        print("read characteristic. value: \(message)")
        switch characteristic.uuid {
        case bluetooth.osCharacteristicUuid:
            self.output(msg: "Peer OS: \(message)")
            // tell peer our OS
            bluetooth.write(message: "mac", characteristic: characteristic)
            // putting this after writing our OS so other side can cancel too
            if message == "mac" || message == "ios" {
                self.output(msg: appleToAppleHotspotErrorMessage)
                // Two Apple devices: hotspot mode can't work. Tearing down now would cancel
                // the BT connection and drop the OS write above, so the peer would never
                // learn it's Apple-to-Apple and would sit silent. Flag the abort instead and
                // tear down in didWriteValueFor, once the write has been acknowledged (it's a
                // .withResponse write, so that callback means the peer has our OS).
                blockingAppleToApple = true
            }
            break
        case bluetooth.ssidCharacteristicUuid:
            let message = String(decoding: characteristic.value!, as: UTF8.self)
            self.output(msg: "Peer SSID: \(message)")
            switch message {
            case "", bluetooth.noSsid:
                // "" (an Android host whose hotspot isn't up yet) or NONE (a Windows host
                // whose main thread hasn't generated credentials yet — our read can race it
                // right after the OS exchange) both mean the credentials don't exist yet:
                // wait and read again. NONE used to be taken as the shared-network-mode
                // marker and fell through to reading the password, but BLE is hotspot-only
                // now (shared-mode BLE was removed in b7e9b59), so a NONE here can only be
                // a not-ready host — proceeding read an empty password and started a
                // transfer that could never work. scheduled rather than slept: this
                // callback runs on the bluetooth queue, and sleeping here stalls every
                // other callback behind it
                queue.asyncAfter(deadline: .now() + 1) {
                    if let ssidCharacteristic = self.bluetooth.ssidCharacteristic {
                        self.bluetooth.read(characteristic: ssidCharacteristic)
                    }
                }
                break
            default:
                self.transfer.ssid = message
                if bluetooth.passwordCharacteristic != nil {
                    bluetooth.read(characteristic: bluetooth.passwordCharacteristic!)
                }
                break
            }
            break
        case bluetooth.passwordCharacteristicUuid:
            self.output(msg: "Peer password: \(message)")
            // hop to main: transfer.task is read there by cleanUpTransfer, and a
            // re-paired peer re-delivers the exchange — don't start a second
            // transfer over the same Transfer state
            DispatchQueue.main.async {
                guard self.transfer.task == nil else {
                    print("Transfer already started, ignoring duplicate password delivery")
                    return
                }
                self.transfer.password = message
                self.transfer.task = Task {
                    await self.transfer.runTransfer()
                }
            }
            break
        default:
            print("Other characteristic: \(characteristic)")
            break
        }
    }

    func peripheral(_ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: (any Error)?) {
        // Apple-to-Apple hotspot abort: our OS write has reached the peer (this callback
        // means it was delivered or errored), so it's safe to tear down now without dropping
        // it. Skip the normal post-write step, which would read the peer's SSID and cascade
        // into starting a transfer that can't succeed.
        if blockingAppleToApple {
            self.transfer.cleanUpTransfer()
            return
        }
        bluetooth.didWriteValue(peripheral: peripheral, characteristic: characteristic, error: error)
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: (any Error)?) {
        print(bluetooth.didDiscoverCharacteristics(peripheral: peripheral, service: service, error: error))
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: (any Error)?) {
        bluetooth.didDiscoverServices(peripheral: peripheral, didDiscoverServices: error)
    }

    func peripheral(_ peripheral: CBPeripheral, didModifyServices invalidatedServices: [CBService]) {
        bluetooth.didModifyServices(peripheral: peripheral, invalidatedServices: invalidatedServices)
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        bluetooth.centralManager(central, didConnect: peripheral)
    }

    func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: (any Error)?) {
        // CBError 14 (peerRemovedPairingInformation): macOS still holds a bond the peer no
        // longer has. This is the recurring macOS<->Linux failure — Linux (BlueZ) doesn't
        // persist the bond across transfers the way macOS caches it, and CoreBluetooth has
        // no API to clear its side, so the transfer can't recover on its own. Tell the user
        // how to reset it instead of surfacing the raw error.
        if let error = error as NSError?,
           error.domain == CBErrorDomain,
           error.code == CBError.peerRemovedPairingInformation.rawValue {
            let name = peripheral.name ?? "the other device"
            output(msg: "Bluetooth pairing with \(name) is out of date (it removed its pairing information). Open System Settings > Bluetooth, remove \(name), then start the transfer again. You can also turn off the \"Use Bluetooth\" switch on both devices and enter the password manually.")
        } else {
            output(msg: "Failed to connect: \(String(describing: error))")
        }
        self.transfer.cleanUpTransfer()
    }

    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        print("Central state: " + bluetooth.centralManagerDidUpdateState(central))
        toggleBluetoothUI(state: central.state)
    }

    func centralManager(
        _ central: CBCentralManager,
        didDiscover peripheral: CBPeripheral,
        advertisementData: [String : Any],
        rssi RSSI: NSNumber
    ) {
        output(msg: "Discovered peripheral")
        DispatchQueue.main.async {
            self.bluetoothIcon.contentTintColor = NSColor.systemBlue
        }
        bluetooth.discoveredPeripheral = peripheral
        bluetooth.discoveredPeripheral?.delegate = self
        bluetooth.didDiscoverPeripheral(didDiscover: peripheral, advertisementData: advertisementData, rssi: RSSI)
    }
}

// Renders `string` as a QR code image, sized to `size` points square. Used in shared
// network mode to display the receiver's password for the sender to scan. Returns nil if
// the QR generator is unavailable or the string can't be encoded.
func qrCodeImage(from string: String, size: CGFloat) -> NSImage? {
    guard let filter = CIFilter(name: "CIQRCodeGenerator") else { return nil }
    filter.setValue(Data(string.utf8), forKey: "inputMessage")
    filter.setValue("M", forKey: "inputCorrectionLevel")
    guard let output = filter.outputImage, output.extent.width > 0 else { return nil }
    // the generator emits one pixel per module; scale up so the modules stay crisp when
    // shown at `size`
    let scale = size / output.extent.width
    let scaled = output.transformed(by: CGAffineTransform(scaleX: scale, y: scale))
    let rep = NSCIImageRep(ciImage: scaled)
    let image = NSImage(size: rep.size)
    image.addRepresentation(rep)
    return image
}

