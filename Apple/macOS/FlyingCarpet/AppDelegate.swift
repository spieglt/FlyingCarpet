//
//  AppDelegate.swift
//  FlyingCarpet
//
//  Created by Theron on 6/9/24.
//

import Cocoa

@main
class AppDelegate: NSObject, NSApplicationDelegate {




    func applicationDidFinishLaunching(_ aNotification: Notification) {
        // Insert code here to initialize your application
    }

    func applicationWillTerminate(_ aNotification: Notification) {
        // Insert code here to tear down your application
    }

    func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
        return true
    }

    var aboutWindow: NSWindow?

    // Shows a resizable About window: a scrollable text view holding the app icon,
    // name, version, aboutMessage, and copyright. Targeted by both the About menu
    // item (via the responder chain) and the window's About button.
    @IBAction func showAbout(_ sender: Any?) {
        if aboutWindow == nil {
            let scrollView = NSTextView.scrollableTextView()
            let textView = scrollView.documentView as! NSTextView
            textView.isEditable = false
            textView.textContainerInset = NSSize(width: 18, height: 18)
            textView.textStorage?.setAttributedString(aboutText())

            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 500, height: 600),
                styleMask: [.titled, .closable, .miniaturizable, .resizable],
                backing: .buffered,
                defer: false
            )
            window.title = "About Flying Carpet"
            window.contentView = scrollView
            window.minSize = NSSize(width: 320, height: 240)
            // we keep a reference and reorder the same window front on later clicks
            window.isReleasedWhenClosed = false
            window.center()
            window.setFrameAutosaveName("AboutWindow")
            aboutWindow = window
        }
        aboutWindow?.makeKeyAndOrderFront(sender)
    }

    // icon + name + version header, aboutMessage with clickable links, copyright
    // footer; version and copyright come from Info.plist
    func aboutText() -> NSAttributedString {
        let centered = NSMutableParagraphStyle()
        centered.alignment = .center
        let text = NSMutableAttributedString()

        let icon = NSTextAttachment()
        icon.image = NSApp.applicationIconImage
        icon.bounds = NSRect(x: 0, y: 0, width: 96, height: 96)
        let iconLine = NSMutableAttributedString(attachment: icon)
        iconLine.addAttribute(.paragraphStyle, value: centered, range: NSRange(location: 0, length: iconLine.length))
        text.append(iconLine)

        text.append(NSAttributedString(string: "\nFlying Carpet\n", attributes: [
            .font: NSFont.boldSystemFont(ofSize: 24),
            .foregroundColor: NSColor.labelColor,
            .paragraphStyle: centered,
        ]))

        let info = Bundle.main.infoDictionary
        let version = info?["CFBundleShortVersionString"] as? String ?? ""
        text.append(NSAttributedString(string: "Version \(version)\n\n", attributes: [
            .font: NSFont.systemFont(ofSize: NSFont.systemFontSize),
            .foregroundColor: NSColor.secondaryLabelColor,
            .paragraphStyle: centered,
        ]))

        let body = NSMutableAttributedString(string: aboutMessage, attributes: [
            .font: NSFont.systemFont(ofSize: NSFont.systemFontSize),
            .foregroundColor: NSColor.labelColor,  // adapts to dark mode
        ])
        if let detector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.link.rawValue) {
            let fullRange = NSRange(aboutMessage.startIndex..., in: aboutMessage)
            for match in detector.matches(in: aboutMessage, range: fullRange) {
                if let url = match.url {
                    body.addAttribute(.link, value: url, range: match.range)
                }
            }
        }
        text.append(body)

        if let copyright = info?["NSHumanReadableCopyright"] as? String {
            text.append(NSAttributedString(string: "\n\n\(copyright)", attributes: [
                .font: NSFont.systemFont(ofSize: NSFont.smallSystemFontSize),
                .foregroundColor: NSColor.secondaryLabelColor,
            ]))
        }
        return text
    }

    let aboutMessage = """
https://flyingcarpet.spiegl.dev
theron@spiegl.dev

Flying Carpet transfers files between two Android, iOS, Linux, macOS, and Windows devices: either over ad hoc WiFi with one device acting as a hotspot, or over a WiFi or wired network the devices already share. In hotspot mode, no access point or internet connection is required, just two devices with WiFi cards in close range.

Apple does not allow hotspots to be started programmatically, so Apple-to-Apple transfers (between macOS and iOS devices) require Shared Network mode. The shared network can itself be a Personal Hotspot made manually on one device and joined from the other before starting the transfer.

INSTRUCTIONS

Select Sending on one device and Receiving on the other, and select the same connection mode (Hotspot or Shared Network) on both.

Hotspot mode: Turn Bluetooth on or off on both devices. If one side fails to initialize Bluetooth or has it turned off, the other side must disable the "Use Bluetooth" switch in Flying Carpet. If not using Bluetooth, select the operating system of the other device.

Shared network mode: Join both devices to the same WiFi or wired network first. Bluetooth and the other device's operating system don't matter in this mode.

Click the "Start Transfer" button on each device. On the sending device, select the files or folder to send. On the receiving device, select the folder in which to receive files. (To send a folder, just choose it in the file picker instead of selecting files. A folder you send is recreated inside the destination folder on the receiving device, with its contents inside it.)

In hotspot mode, if using Bluetooth, confirm the 6-digit PIN on each side. The WiFi connection will be configured automatically. If not using Bluetooth, you will need to scan a QR code or type in a password. (If transferring between Android and macOS, you will have to type in the SSID and password.) When prompted to join a WiFi network or modify WiFi settings, say Allow. You may have to grant location permissions, which Apple requires to scan for WiFi networks. Flying Carpet does not read or collect your location.

In shared network mode, the receiving device displays a password: enter it on the sending device when prompted, and the devices will find each other on the network automatically. If asked to allow Flying Carpet to find devices on the local network, say Allow.

TROUBLESHOOTING

Android devices must be kept awake with Flying Carpet in the foreground for the duration of the transfer, or the WiFi connection may drop.

If using Bluetooth fails, try manually unpairing the devices from one another and starting a new transfer.

Flying Carpet may make multiple attempts to join the other device's hotspot.

In shared network mode, if the devices can't find each other, make sure both are on the same network and that the password was entered correctly. Some public or guest networks don't allow devices on them to communicate with each other.

Licensed under the GPL3: https://www.gnu.org/licenses/gpl-3.0.html#license-text

Thanks for using it and please provide feedback on the App Store or GitHub!
"""
}

