//
//  QRScanner.swift
//  FlyingCarpet
//
//  Created by Theron on 9/27/22.
//

import Foundation
import AVFoundation
import CoreImage
import UIKit

protocol ScannerViewControllerDelegate: AnyObject {
    func codeScanned(result: String)
    func scanCancelled()
}

class ScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var captureSession: AVCaptureSession!
    var previewLayer: AVCaptureVideoPreviewLayer!
    weak var delegate: ScannerViewControllerDelegate?
    var codeFound = false

    @IBOutlet weak var QRLabel: UITextView!
    @IBOutlet weak var QRIcon: UIImageView!
    @IBOutlet weak var QRView: UIView!

    override func viewDidLoad() {
        super.viewDidLoad()

        view.backgroundColor = UIColor.black
        self.captureSession = AVCaptureSession()

        guard let videoCaptureDevice = AVCaptureDevice.default(for: .video) else { return }
        let videoInput: AVCaptureDeviceInput

        do {
            videoInput = try AVCaptureDeviceInput(device: videoCaptureDevice)
        } catch {
            return
        }

        if (self.captureSession.canAddInput(videoInput)) {
            self.captureSession.addInput(videoInput)
        } else {
            failed()
            return
        }

        let metadataOutput = AVCaptureMetadataOutput()

        if (self.captureSession.canAddOutput(metadataOutput)) {
            self.captureSession.addOutput(metadataOutput)

            metadataOutput.setMetadataObjectsDelegate(self, queue: DispatchQueue.main)
            metadataOutput.metadataObjectTypes = [.qr]
        } else {
            failed()
            return
        }

        previewLayer = AVCaptureVideoPreviewLayer(session: self.captureSession)
        previewLayer.frame = view.layer.bounds
        previewLayer.videoGravity = .resizeAspectFill
        view.layer.addSublayer(previewLayer)
        
        view.bringSubviewToFront(QRIcon)
        view.bringSubviewToFront(QRLabel)
        view.bringSubviewToFront(QRView)

        DispatchQueue.global(qos: .background).async {
            self.captureSession.startRunning()
        }
    }

    func failed() {
        let ac = UIAlertController(title: "Scanning not supported", message: "Your device does not support scanning a code from an item. Please use a device with a camera.", preferredStyle: .alert)
        ac.addAction(UIAlertAction(title: "OK", style: .default))
        present(ac, animated: true)
        self.captureSession = nil
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        codeFound = false
        if (self.captureSession?.isRunning == false) {
            DispatchQueue.global(qos: .background).async {
                self.captureSession.startRunning()
            }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        if !codeFound {
            self.delegate?.scanCancelled()
        }
        if (self.captureSession?.isRunning == true) {
            self.captureSession.stopRunning()
        }
    }

    func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput metadataObjects: [AVMetadataObject], from connection: AVCaptureConnection) {
        self.captureSession.stopRunning()

        if let metadataObject = metadataObjects.first {
            guard let readableObject = metadataObject as? AVMetadataMachineReadableCodeObject else { return }
            guard let stringValue = readableObject.stringValue else { return }
            AudioServicesPlaySystemSound(SystemSoundID(kSystemSoundID_Vibrate))
            found(code: stringValue)
        }

        dismiss(animated: true)
    }

    func found(code: String) {
        codeFound = true
        self.delegate?.codeScanned(result: code)
    }

    override var prefersStatusBarHidden: Bool {
        return true
    }

    override var supportedInterfaceOrientations: UIInterfaceOrientationMask {
        return .portrait
    }

    private func updatePreviewLayer(captureConnection: AVCaptureConnection, orientation: AVCaptureVideoOrientation) {
        captureConnection.videoOrientation = orientation
        previewLayer?.frame = view.bounds
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()

        if let connection = previewLayer?.connection {
            let currentDevice = UIDevice.current
            let orientation: UIDeviceOrientation = currentDevice.orientation
            let previewLayerConnection: AVCaptureConnection = connection

            if previewLayerConnection.isVideoOrientationSupported {
                switch orientation {
                case .portrait: self.updatePreviewLayer(captureConnection: previewLayerConnection, orientation: .portrait)
                case .landscapeRight: self.updatePreviewLayer(captureConnection: previewLayerConnection, orientation: .landscapeLeft)
                case .landscapeLeft: self.updatePreviewLayer(captureConnection: previewLayerConnection, orientation: .landscapeRight)
                case .portraitUpsideDown: self.updatePreviewLayer(captureConnection: previewLayerConnection, orientation: .portraitUpsideDown)
                default: self.updatePreviewLayer(captureConnection: previewLayerConnection, orientation: .portrait)
                }
            }
        }
    }
}

// Renders `string` as a QR code image, sized to `size` points square. Used in shared
// network mode to display the receiver's password for a sender to scan. Returns nil if the
// QR generator is unavailable or the string can't be encoded.
func qrCodeImage(from string: String, size: CGFloat) -> UIImage? {
    guard let filter = CIFilter(name: "CIQRCodeGenerator") else { return nil }
    filter.setValue(Data(string.utf8), forKey: "inputMessage")
    filter.setValue("M", forKey: "inputCorrectionLevel")
    guard let output = filter.outputImage, output.extent.width > 0 else { return nil }
    // the generator emits one pixel per module; scale up so the modules stay crisp
    let scale = size / output.extent.width
    let scaled = output.transformed(by: CGAffineTransform(scaleX: scale, y: scale))
    guard let cgImage = CIContext().createCGImage(scaled, from: scaled.extent) else { return nil }
    return UIImage(cgImage: cgImage)
}

// Shown to a shared network receiver so a sender can scan the generated password as a QR
// code (the QR content is the bare password, matching the desktop and Android receivers)
// instead of typing it. The password is also shown as text for a sender that can't scan
// (e.g. macOS). The transfer runs underneath this screen; dismissing it doesn't cancel it.
class QRDisplayViewController: UIViewController {
    var password = ""

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground

        let instructions = UILabel()
        instructions.text = "Start the transfer on the sending device, then scan this QR code (Android or iOS) or enter the password below."
        instructions.numberOfLines = 0
        instructions.textAlignment = .center
        instructions.font = .systemFont(ofSize: 16)

        let imageView = UIImageView(image: qrCodeImage(from: password, size: 480))
        imageView.contentMode = .scaleAspectFit
        // keep the modules crisp if the image view is larger than the generated image
        imageView.layer.magnificationFilter = .nearest

        let passwordLabel = UILabel()
        passwordLabel.text = password
        passwordLabel.textAlignment = .center
        passwordLabel.font = .monospacedSystemFont(ofSize: 22, weight: .semibold)

        let doneButton = UIButton(type: .system)
        doneButton.setTitle("Done", for: .normal)
        doneButton.titleLabel?.font = .systemFont(ofSize: 18)
        doneButton.addTarget(self, action: #selector(dismissSelf), for: .touchUpInside)

        let stack = UIStackView(arrangedSubviews: [instructions, imageView, passwordLabel, doneButton])
        stack.axis = .vertical
        stack.spacing = 24
        stack.alignment = .fill
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)

        NSLayoutConstraint.activate([
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 32),
            stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -32),
            imageView.heightAnchor.constraint(equalTo: imageView.widthAnchor),
        ])
    }

    @objc func dismissSelf() {
        dismiss(animated: true)
    }
}
