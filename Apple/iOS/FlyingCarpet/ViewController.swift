//
//  ViewController.swift
//  FlyingCarpet
//
//  Created by Theron on 5/25/22.
//

import CoreBluetooth
import Network
import NetworkExtension
import PhotosUI
import UIKit
import UniformTypeIdentifiers

class ViewController:
    UIViewController,
    UIDocumentPickerDelegate,
    PHPickerViewControllerDelegate,
    CBCentralManagerDelegate,
    CBPeripheralDelegate,
    CBPeripheralManagerDelegate,
    ScannerViewControllerDelegate,
    Transfer.Delegate
{

    @IBOutlet weak var startTransferButton: UIButton!
    @IBOutlet weak var cancelButton: UIButton!
    @IBOutlet weak var modeSwitch: UISegmentedControl!
    @IBOutlet weak var connectionModeSwitch: UISegmentedControl!  // Hotspot / Shared Network
    @IBOutlet weak var bluetoothSwitch: UISwitch!
    @IBOutlet weak var bluetoothIcon: UIImageView!
    @IBOutlet weak var outputField: UITextView!
    @IBOutlet weak var progressBar: UIProgressView!
    @IBOutlet weak var logoImageView: UIImageView!  // shows the QR code while receiving in shared network mode
    var transfer = Transfer()
    var folderPicker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder])
    var filePicker = UIDocumentPickerViewController(forOpeningContentTypes: [UTType.init("public.data")!])
    let bluetooth = Bluetooth()
    let queue = DispatchQueue(label: "bluetooth")
    // set when we detect the BT peer is another Apple device in hotspot mode; suppresses
    // the rest of the BT exchange while we notify the peer and tear down (see didWriteValueFor)
    var blockingAppleToApple = false
    var outputText = NSMutableAttributedString(string: "Android, Linux, macOS, and Windows versions are available at https://flyingcarpet.spiegl.dev.", attributes: [.font: UIFont.systemFont(ofSize: 14), .foregroundColor: UIColor.label])

    override func viewDidLoad() {
        super.viewDidLoad()
        // Do any additional setup after loading the view.
        self.folderPicker.delegate = self
        self.filePicker.delegate = self
        self.filePicker.allowsMultipleSelection = true
        
        // trigger the local network permission prompt early. no wait: nothing here
        // depends on the result, and blocking launch on a network probe risks a
        // watchdog kill if it never resolves
        let _ = LocalNetworkPermissionTester(semaphore: DispatchSemaphore(value: 0))

        // initialize bluetooth
        initializeBluetooth()

        // Sweep any camera-roll copies left in Documents by a run that crashed or was
        // force-quit before cleanUpTransfer(). They're only ever needed for the duration
        // of the transfer that copied them. Console-only: there's nothing for the user to
        // act on, and startTransfer() sweeps again anyway.
        do {
            try emptyDocsDir()
        } catch {
            print("Error emptying documents directory at launch: \(error)")
        }

        // Remove any leftover flyingCarpet_ hotspot configurations from a previous run
        // that crashed or was force-quit before cleanup. joinOnce is false (it stopped
        // working on recent iOS), so these configurations otherwise persist system-wide.
        NEHotspotConfigurationManager.shared.getConfiguredSSIDs { ssids in
            for ssid in ssids where ssid.hasPrefix("flyingCarpet_") {
                NEHotspotConfigurationManager.shared.removeConfiguration(forSSID: ssid)
            }
        }
    }

    func output(msg: String) {
        DispatchQueue.main.async {
            if msg.count >= 18, msg[msg.startIndex ..< msg.index(msg.startIndex, offsetBy: 18)] == "shareddocuments://" {
                let displayText = "\nClick here to open receiving folder"
                let link = NSMutableAttributedString(string: displayText, attributes: [.font: UIFont.systemFont(ofSize: 14), .link: msg])
                self.outputText.append(link)
            } else {
                self.outputText.append(NSMutableAttributedString(string: "\n" + msg, attributes: [.font: UIFont.systemFont(ofSize: 14), .foregroundColor: UIColor.label]))
            }
            self.outputField.attributedText = self.outputText
            let range = NSRange(location: self.outputField.text.count - 1, length: 0)
            self.outputField.scrollRangeToVisible(range)
        }
    }

    func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        // drop the delegate so a duplicate delivery (e.g. a double-tap on Add) can't run this twice
        picker.delegate = nil
        picker.dismiss(animated: true)
        if results.count == 0 {
            self.output(msg: "User cancelled photo/video selection.")
            self.transfer.cleanUpTransfer()
            return
        }
        let docsDir = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let fileCopySemaphore = DispatchSemaphore.init(value: 0)
        // the item provider completions run concurrently on background queues, so
        // appends to the shared file list must be serialized
        let fileListLock = NSLock()
        // Reserves a collision-free destination in docsDir for `filename` (guarded by
        // fileListLock, since resource completions run concurrently). If two picked items
        // resolve to the same name, later ones get "-2", "-3", … before the extension, so no
        // write silently clobbers another — which would drop a file from the transfer while
        // the receiver is still told to expect it.
        var usedNames = Set<String>()
        func reserveURL(_ filename: String) -> URL {
            fileListLock.lock()
            defer { fileListLock.unlock() }
            let ext = (filename as NSString).pathExtension
            let stem = (filename as NSString).deletingPathExtension
            var candidate = filename
            var counter = 2
            while usedNames.contains(candidate) {
                candidate = ext.isEmpty ? "\(stem)-\(counter)" : "\(stem)-\(counter).\(ext)"
                counter += 1
            }
            usedNames.insert(candidate)
            return docsDir.appendingPathComponent(candidate)
        }
        for result in results {
            print("result: \(result)")

            // A Live Photo is a still plus a paired video that share a hidden content
            // identifier. Send both, byte-for-byte (no re-encode), under their original
            // filenames: an Apple receiver re-pairs them into a Live Photo, and a non-Apple
            // receiver just sees a photo and a short video. Passthrough is required —
            // re-encoding would strip the content identifier (breaking re-pairing) and the
            // EXIF/GPS metadata. Prefer the edited render (.fullSize*) so the recipient gets
            // what the user sees, falling back to the original capture.
            if result.itemProvider.canLoadObject(ofClass: PHLivePhoto.self) {
                result.itemProvider.loadObject(ofClass: PHLivePhoto.self) { livePhoto, error in
                    guard let livePhoto = livePhoto as? PHLivePhoto else {
                        self.output(msg: "Could not read Live Photo: \(error?.localizedDescription ?? "unknown error").")
                        fileCopySemaphore.signal()
                        return
                    }
                    let resources = PHAssetResource.assetResources(for: livePhoto)
                    func preferred(_ edited: PHAssetResourceType, _ original: PHAssetResourceType) -> PHAssetResource? {
                        resources.first { $0.type == edited } ?? resources.first { $0.type == original }
                    }
                    let wanted = [
                        preferred(.fullSizePhoto, .photo),
                        preferred(.fullSizePairedVideo, .pairedVideo),
                    ].compactMap { $0 }

                    // Signal the semaphore once per picker result — after every constituent
                    // file is written — so the wait loop below stays balanced even though this
                    // one result yields two files. group.notify still fires (immediately) if
                    // `wanted` is empty, so a malformed asset can't hang the wait.
                    let group = DispatchGroup()
                    for resource in wanted {
                        group.enter()
                        let resourceData = NSMutableData()
                        let options = PHAssetResourceRequestOptions()
                        options.isNetworkAccessAllowed = true // the resource may live only in iCloud
                        PHAssetResourceManager.default().requestData(for: resource, options: options, dataReceivedHandler: { data in
                            resourceData.append(data)
                        }, completionHandler: { error in
                            defer { group.leave() }
                            if let error = error {
                                self.output(msg: "Error reading \(resource.originalFilename): \(error).")
                                return
                            }
                            let fileURL = reserveURL(resource.originalFilename)
                            do {
                                try (resourceData as Data).write(to: fileURL)
                            } catch {
                                self.output(msg: "Error writing \(resource.originalFilename) to app's documents directory: \(error).")
                                return
                            }
                            fileListLock.lock()
                            self.transfer.fileList.append(fileURL)
                            fileListLock.unlock()
                            print("Wrote to \(fileURL).") // terminal only; UI gets a single summary below
                        })
                    }
                    group.notify(queue: .global()) {
                        fileCopySemaphore.signal()
                    }
                }
            } else if result.itemProvider.hasItemConformingToTypeIdentifier(UTType.item.identifier) {
                // problem is that this function is async
                result.itemProvider.loadFileRepresentation(forTypeIdentifier: UTType.item.identifier) { url, error in
                    defer { fileCopySemaphore.signal() }
                    if let url = url {
                        let dstPath = reserveURL(url.lastPathComponent)

                        do {
                            // temp storage is ours alone and is swept at launch and before
                            // every transfer, so anything still at this path is stale; replace
                            // it rather than letting copyItem fail with "file exists". (The
                            // Live Photo path above gets this for free: Data.write truncates.)
                            try? FileManager.default.removeItem(at: dstPath)
                            try FileManager.default.copyItem(atPath: url.path, toPath: dstPath.path)
                            fileListLock.lock()
                            self.transfer.fileList.append(dstPath)
                            fileListLock.unlock()
                            let size = try getFileSize(file: dstPath)
                            print("file size: \(size)")
                        } catch {
                            self.output(msg: "Error copying file to temp storage: \(error).")
                            self.transfer.cleanUpTransfer()
                            return
                        }
                        print("copied file from \(url.path) to \(dstPath.path)")
                    } else {
                        self.output(msg: "Error: could not get filename from path.")
                        self.transfer.cleanUpTransfer()
                        return
                    }
                }
            } else {
                self.output(msg: "File did not conform to UTType.item.")
                fileCopySemaphore.signal() // still balance the wait loop; just skip this item
            }
        }
        
        // wait for the copies off the main thread: blocking here kept the picker's
        // dismissal (and all output) frozen until the copies finished
        self.output(msg: "Copying files from camera roll to temp storage...")
        DispatchQueue.global().async {
            for _ in 0 ..< results.count {
                fileCopySemaphore.wait()
                print("Copied item") // per-item detail to the terminal, not the UI
            }
            self.output(msg: "Copy complete.")

            DispatchQueue.main.async {
                if self.transfer.connectionMode == .sharedNetwork {
                    // Shared network mode doesn't use Bluetooth; exchange the password manually.
                    self.startSharedNetworkManual()
                    return
                }
                if self.bluetooth.active.value {
                    if self.transfer.mode == .Sending {
                        // Hotspot mode: sender generates password for the hotspot
                        let password = generatePassword()
                        self.transfer.password = password
                        self.bluetooth.localPassword = password
                        self.bluetooth.startAdvertising()
                    } else {
                        self.output(msg: "Starting Bluetooth scan, waiting for sending device...")
                        self.bluetooth.scan()
                    }
                } else {
                    self.performSegue(withIdentifier: "goToQRScanner", sender: self)
                }
            }
        }
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        // enumerating a large folder can take a while; keep it off the main thread
        DispatchQueue.global().async {
            do {
                try self.transfer.handleFileSelection(urls: urls)
            } catch {
                self.output(msg: "Could not read contents files chosen: \(error)")
                self.transfer.cleanUpTransfer()
                return
            }

            DispatchQueue.main.async {
                print("bluetooth active: \(self.bluetooth.active.value)")
                if self.transfer.connectionMode == .sharedNetwork {
                    // Shared network mode doesn't use Bluetooth; exchange the password manually.
                    self.startSharedNetworkManual()
                    return
                }
                if self.bluetooth.active.value {
                    if self.transfer.mode == .Sending {
                        // Hotspot mode: sender generates password for the hotspot
                        let password = generatePassword()
                        self.transfer.password = password
                        self.bluetooth.localPassword = password
                        self.bluetooth.startAdvertising()
                    } else {
                        self.output(msg: "Starting Bluetooth scan, waiting for sending device...")
                        self.bluetooth.scan()
                    }
                } else {
                    self.performSegue(withIdentifier: "goToQRScanner", sender: self)
                }
            }
        }
    }
    
    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        self.output(msg: "User cancelled file/folder selection.")
        self.transfer.cleanUpTransfer()
    }

    func getFileChooser() -> UIAlertController {
        let alertController = UIAlertController(title: "Send from:", message: "Choose whether to send from the Files App, Camera Roll, or send a folder.", preferredStyle: .alert)
        alertController.addAction(UIAlertAction(title: "Files App", style: .default) { _ in
            self.present(self.filePicker, animated: true, completion: nil)
        })
        alertController.addAction(UIAlertAction(title: "Camera Roll", style: .default) { _ in
            var config = PHPickerConfiguration()
            config.selectionLimit = 0
            config.filter = PHPickerFilter.any(of: [PHPickerFilter.images, PHPickerFilter.videos])

            let pickerViewController = PHPickerViewController(configuration: config)
            pickerViewController.delegate = self
            self.present(pickerViewController, animated: true, completion: nil)
        })
        alertController.addAction(UIAlertAction(title: "Send Folder", style: .default) { _ in
            self.transfer.sendFolder = true
            self.present(self.folderPicker, animated: true, completion: nil)
        })
        alertController.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in
            self.output(msg: "User cancelled.")
            self.transfer.cleanUpTransfer()
        })
        return alertController
    }

    override func prepare(for segue: UIStoryboardSegue, sender: Any?) {
        if segue.identifier == "goToQRScanner" {
            let destination = segue.destination as! ScannerViewController
            destination.delegate = self
        }
    }

    func codeScanned(result: String) {
        let components = Array(result.split(separator: ";"))
        if components.count > 1 {
            // must be joining Android ad hoc network, need SSID and password
            self.transfer.ssid = String(components[0])
            self.transfer.password = String(components[1])
        } else {
            // joining Windows or Linux, just received password
            self.transfer.password = String(components[0])
        }
        self.transfer.task = Task {
            await self.transfer.runTransfer()
        }
    }

    func scanCancelled() {
        self.output(msg: "QR code scanning cancelled")
        self.transfer.cleanUpTransfer()
    }

    // Shared network mode password exchange (no Bluetooth): the receiver generates and
    // displays the password; the sender types in the password shown on the receiver.
    func startSharedNetworkManual() {
        if transfer.mode == .Receiving {
            let password = generatePassword()
            transfer.password = password
            self.output(msg: "Password: \(password)")
            self.output(msg: "Enter this password on the sending device, or have it scan the QR code.")
            // replace the logo on the main screen with the QR code (like the other
            // platforms) so a sender can scan it after this popup is dismissed. reset to the
            // logo in toggleUI(transferRunning: false) when the transfer ends.
            self.logoImageView.image = qrCodeImage(from: password, size: 480)
            // show the password as text and as a QR code the sender can scan (Android or
            // iOS). the transfer starts underneath; discovery doesn't wait for the dismissal.
            let qrVC = QRDisplayViewController()
            qrVC.password = password
            self.present(qrVC, animated: true)
            self.transfer.task = Task {
                await self.transfer.runTransfer()
            }
        } else {
            promptForPassword { password in
                self.transfer.password = password
                self.transfer.task = Task {
                    await self.transfer.runTransfer()
                }
            }
        }
    }

    func promptForPassword(
        message: String = "Enter the password shown on the receiving device, or scan its QR code.",
        completion: @escaping (String) -> Void
    ) {
        let alert = UIAlertController(
            title: "Enter Password",
            message: message,
            preferredStyle: .alert
        )
        alert.addTextField { textField in
            textField.placeholder = "Password"
            textField.autocapitalizationType = .none
            textField.autocorrectionType = .no
        }
        alert.addAction(UIAlertAction(title: "Start", style: .default) { _ in
            let password = (alert.textFields?.first?.text ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if password.count < 10 {
                // generated passwords are always 10 characters, so this must be a typo
                self.promptForPassword(
                    message: "Password must be at least 10 characters. Enter the password shown on the receiving device, or scan its QR code.",
                    completion: completion
                )
                return
            }
            completion(password)
        })
        alert.addAction(UIAlertAction(title: "Scan QR Code", style: .default) { _ in
            // the scanner delivers its result to codeScanned(result:), which sets the
            // password and starts the transfer. A shared network QR code is the bare
            // password, so codeScanned's single-component path handles it.
            self.performSegue(withIdentifier: "goToQRScanner", sender: self)
        })
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in
            self.output(msg: "Cancelled.")
            self.transfer.cleanUpTransfer()
        })
        self.present(alert, animated: true)
    }

    @IBAction func startTransfer(sender: UIButton) {
        // check for network permission. bounded wait: this check exists to show a
        // helpful message when the permission is denied, which the probe reports
        // quickly — an unresolved probe shouldn't freeze the app, so on timeout we
        // proceed as if permitted
        let semaphore = DispatchSemaphore(value: 0)
        let tester = LocalNetworkPermissionTester(semaphore: semaphore)
        let probeResult = semaphore.wait(timeout: .now() + 3)
        if probeResult == .success && !tester.success {
            self.output(msg: "Flying Carpet does not have Local Network permissions, which it requires to transfer data with other devices. If you would like to enable this permission, please go to \"Settings\" > \"Privacy & Security\" > \"Local Network\" and turn on the toggle for Flying Carpet.")
            return
        }

        self.toggleUI(transferRunning: true)

        // reset the Apple-to-Apple abort flag for this run
        self.blockingAppleToApple = false

        // refresh transfer
        self.transfer = Transfer()
        transfer.delegate = self

        // Set connection mode from UI
        if connectionModeSwitch.selectedSegmentIndex == 0 {
            transfer.connectionMode = .hotspot
        } else {
            transfer.connectionMode = .sharedNetwork
        }

        // ensure no duplicates, in case cleanUpTransfer() somehow didn't run previously.
        do {
            try emptyDocsDir()
        } catch {
            self.output(msg: "Error emptying temporary camera roll contents from app's documents directory: \(error).")
        }

        // send or receive
        if self.modeSwitch.selectedSegmentIndex == 0 {
            self.transfer.mode = .Sending

            // choose from files app or camera roll
            let alertController = getFileChooser()
            self.present(alertController, animated: true)
        } else {
            self.transfer.mode = .Receiving
            self.present(self.folderPicker, animated: true, completion: nil)
        }
    }
    
    @IBAction func modeToggled(_ sender: Any) {
        if self.modeSwitch.selectedSegmentIndex == 0 {
            self.startTransferButton.setTitle("Select Files", for: .normal)
        } else {
            self.startTransferButton.setTitle("Select Folder", for: .normal)
        }
    }

    @IBAction func connectionModeSwitched(_ sender: UISegmentedControl) {
        if sender.selectedSegmentIndex == 0 {
            transfer.connectionMode = .hotspot
            // re-enable Bluetooth if the hardware supports it
            bluetoothSwitch.isEnabled = bluetoothSwitchShouldBeEnabled()
        } else {
            transfer.connectionMode = .sharedNetwork
            // Bluetooth isn't used in shared network mode (the password is entered
            // manually); turn it off and disable the switch.
            bluetooth.active.value = false
            bluetooth.peripheralManager?.stopAdvertising()
            bluetooth.centralManager?.stopScan()
            bluetoothSwitch.setOn(false, animated: true)
            bluetoothSwitch.isEnabled = false
        }
    }

    @IBAction func cancelTransfer(_ sender: UIButton) {
        // print("transfer cancelled")
        self.transfer.cleanUpTransfer()
    }

    func isConnected() async -> Bool {
        if let current = await NEHotspotNetwork.fetchCurrent() {
            self.output(msg: "Successfully joined \(current.ssid).")
            return true
        } else {
            self.output(msg: "Failed to join other device's network. Retrying.")
            return false
        }
    }

    func joinHotspot() async throws {
        // set up hotspot
        let config = NEHotspotConfiguration(ssid: self.transfer.ssid,
                                            passphrase: self.transfer.password,
                                            isWEP: false)
        // joinOnce configurations stopped reliably joining on recent iOS; use a
        // persistent configuration instead, removed in forgetHotspot() on cleanup
        config.joinOnce = false

        // freeze the "networks we were already on" set before the join starts:
        // association continues after apply() returns, and until DHCP on the hotspot
        // finishes the wifi path still advertises the old gateway
        self.transfer.latchPreJoinGateways()

        // see if we successfully joined hotspot
        while true {
            try await NEHotspotConfigurationManager.shared.apply(config)
            // association continues after apply() returns; give it time to finish
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            if Task.isCancelled {
                throw TransferError.UserCancelled
            }
            if await isConnected() {
                break
            }
            // re-applying an identical configuration doesn't retry the join on
            // recent iOS; remove it so the next apply() makes a fresh attempt
            NEHotspotConfigurationManager.shared.removeConfiguration(forSSID: self.transfer.ssid)
        }

        try await self.transfer.awaitPeerGateway()
    }

    func forgetHotspot() {
        NEHotspotConfigurationManager.shared.removeConfiguration(forSSID: self.transfer.ssid)
    }
    
    func setProgress(_ progress: Float, animated: Bool, hidden: Bool) {
        DispatchQueue.main.async {
            if self.progressBar.isHidden && !hidden {
                self.progressBar.isHidden = false
            } else if !self.progressBar.isHidden && hidden {
                self.progressBar.isHidden = true
            }
            self.progressBar.setProgress(progress, animated: animated)
        }
    }

    func toggleUI(transferRunning: Bool) {
        if transferRunning {
            self.bluetoothSwitch.isEnabled = false
            self.startTransferButton.isHidden = true
            self.cancelButton.isHidden = false
            self.modeSwitch.isEnabled = false
            self.connectionModeSwitch.isEnabled = false
        } else {
            self.bluetoothSwitch.isEnabled = self.bluetoothSwitchShouldBeEnabled()
            self.startTransferButton.isHidden = false
            self.cancelButton.isHidden = true
            self.modeSwitch.isEnabled = true
            self.connectionModeSwitch.isEnabled = true
            // restore the logo, replaced by the QR code while receiving in shared network mode
            self.logoImageView.image = UIImage(named: "Image")
        }
    }

    func emptyDocsDir() throws {
        let fileManager = FileManager.default
        let docsDir = fileManager.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let filenames = try fileManager.contentsOfDirectory(atPath: docsDir.path)
        for filename in filenames {
            print("removing: \(filename)")
            try fileManager.removeItem(atPath: "\(docsDir.path)/\(filename)")
        }
//        let tempDir = fileManager.temporaryDirectory
//        print("tempDir: \(tempDir)")
//        if fileManager.fileExists(atPath: tempDir.path) {
//            try fileManager.removeItem(at: tempDir)
//            print("removed tmp")
//        } else {
//            print("no tmp folder")
//        }
    }
    
    
    // bluetooth

    func initializeBluetooth() {
        // initialize bluetooth peripheral
        bluetooth.peripheralManager = CBPeripheralManager.init(delegate: self, queue: queue)
        // initialize bluetooth central
        bluetooth.centralManager = CBCentralManager(delegate: self, queue: queue)
        self.output(msg: "Bluetooth initialized")
    }

    /// Bluetooth is hotspot-only: shared network mode exchanges the password manually
    /// (display + type/QR), so the switch stays off and greyed out there. Every place that
    /// re-enables the switch must go through this, or the switch comes back live in shared
    /// network mode — which is what happened when a transfer finished and `toggleUI` restored
    /// the controls without consulting the connection mode.
    func bluetoothSwitchShouldBeEnabled() -> Bool {
        return bluetooth.capable.value && transfer.connectionMode == .hotspot
    }

    @IBAction func bluetoothSwitchFlipped(_ sender: UISwitch) {
        if sender.isOn {
            bluetooth.active.value = true
        } else {
            bluetooth.active.value = false
            bluetooth.peripheralManager?.stopAdvertising()
            bluetooth.centralManager?.stopScan()
        }
    }
    
    func toggleBluetoothUI(state: CBManagerState) {
        DispatchQueue.main.async {
            switch state {
            case .unsupported, .unauthorized, .poweredOff:
                self.bluetooth.active.value = false
                self.bluetooth.capable.value = false
                self.bluetoothSwitch.isOn = false
                self.bluetoothSwitch.isEnabled = false
                self.bluetoothIcon.isHidden = true
                self.output(msg: "Bluetooth is off, not present on this system, or Bluetooth permissions were denied.")
                return
            case .poweredOn:
                self.bluetooth.capable.value = true
                // Don't switch Bluetooth on in shared network mode, where it isn't used:
                // turning the radio on in Control Center mid-session lands here.
                if self.transfer.connectionMode == .hotspot {
                    self.bluetooth.active.value = true
                    self.bluetoothSwitch.isOn = true
                    self.bluetoothSwitchFlipped(self.bluetoothSwitch)
                }
                break
            default:
                break
            }
            self.bluetoothSwitch.isEnabled = self.bluetoothSwitchShouldBeEnabled()
            self.bluetoothIcon.isHidden = false
        }
    }

    func stopBluetooth() {
        self.bluetooth.centralManager?.stopScan()
        self.bluetooth.disconnectPeripheral()
        self.bluetooth.peripheralManager?.stopAdvertising()
        self.bluetooth.removeService()
        DispatchQueue.main.async {
            self.bluetoothIcon.tintColor = UIColor.systemGray
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
            self.bluetoothIcon.tintColor = UIColor.systemBlue
        }
        if error != nil {
            output(msg: "Advertising error: \(String(describing: error))")
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
        var message = ""
        for request in requests {
            if let v = request.value {
                message += String(decoding: v, as: UTF8.self)
            }
        }
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
    
    func peripheral(_ peripheral: CBPeripheral, didModifyServices invalidatedServices: [CBService]) {
        bluetooth.didModifyServices(peripheral: peripheral, invalidatedServices: invalidatedServices)
    }

    // TODO: move this to Bluetooth?
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
            bluetooth.write(message: "ios", characteristic: characteristic)
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
                self.output(msg: "Waiting for peer SSID...")
                queue.asyncAfter(deadline: .now() + 2) {
                    if let ssidCharacteristic = self.bluetooth.ssidCharacteristic {
                        self.bluetooth.read(characteristic: ssidCharacteristic)
                    }
                }
                break
            default:
                self.output(msg: "Peer SSID: \(message)")
                self.transfer.ssid = message
                if bluetooth.passwordCharacteristic != nil {
                    bluetooth.read(characteristic: bluetooth.passwordCharacteristic!)
                }
                break
            }
            break
        case bluetooth.passwordCharacteristicUuid:
            // self.output(msg: "Peer password: \(message)")
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

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        output(msg: "Connected to peripheral")
        bluetooth.centralManager(central, didConnect: peripheral)
    }

    func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: (any Error)?) {
        // Mirror of the macOS implementation: without the cleanUpTransfer() the transfer
        // just waited forever after a failed connect, and CBError 14 deserves its
        // explanation. CBError 14 (peerRemovedPairingInformation): iOS still holds a bond
        // the peer no longer has, and CoreBluetooth has no API to clear its side, so the
        // transfer can't recover on its own — tell the user how to reset it.
        if let error = error as NSError?,
           error.domain == CBErrorDomain,
           error.code == CBError.peerRemovedPairingInformation.rawValue {
            let name = peripheral.name ?? "the other device"
            output(msg: "Bluetooth pairing with \(name) is out of date (it removed its pairing information). Open Settings > Bluetooth, remove \(name), then start the transfer again. You can also turn off the \"Use Bluetooth\" switch on both devices and enter the password manually.")
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
        bluetooth.discoveredPeripheral = peripheral
        bluetooth.discoveredPeripheral?.delegate = self
        bluetooth.didDiscoverPeripheral(didDiscover: peripheral, advertisementData: advertisementData, rssi: RSSI)
    }
}
