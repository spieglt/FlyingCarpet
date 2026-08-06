package dev.spiegl.flyingcarpet

import android.app.AlertDialog
import android.app.Dialog
import android.os.Bundle
import androidx.fragment.app.DialogFragment

class Alert(private val message: String) : DialogFragment() {
    override fun onCreateDialog(savedInstanceState: Bundle?): Dialog {
        return activity?.let {
            val builder = AlertDialog.Builder(it)
//            builder.setTitle("Enter on Peer")
                .setMessage(message)
                .setPositiveButton("OK") { _, _ ->
                    // nothing to do here
                }
            builder.create()
        } ?: throw IllegalStateException("Activity cannot be null")
    }
}

class About : DialogFragment() {
    override fun onCreateDialog(savedInstanceState: Bundle?): Dialog {
        return activity?.let {
            val builder = AlertDialog.Builder(it)
                .setTitle("About Flying Carpet")
                .setMessage(AboutMessage.trimIndent())
//                .setPositiveButton("OK") {_, _ -> }
            builder.create()
        } ?: throw IllegalStateException("Activity cannot be null")
    }
}

const val AboutMessage = """
    https://flyingcarpet.spiegl.dev
    Version 10.0.3
    theron@spiegl.dev
    Copyright 2026, Theron Spiegl, all rights reserved.

    Flying Carpet transfers files between two Android, iOS, Linux, macOS, and Windows devices over ad hoc WiFi. In Hotspot mode, no access point or shared network is required, just two WiFi cards in close range. Hotspot mode does not work from one Apple device (macOS or iOS) to another, because Apple no longer allows hotspots to be started programmatically: use Shared Network mode for those transfers.

    In Shared Network mode, both devices must be connected to the same network. No hotspot is created: the devices find each other on the network automatically. Bluetooth is not used in this mode. The receiving device generates and displays a password, which must be entered or scanned on the sending device.

    INSTRUCTIONS

    Turn Bluetooth on or off on both devices. If one side fails to initialize Bluetooth or has it turned off, the other side must disable the "Use Bluetooth" switch in Flying Carpet.
    
    Select Sending on one device and Receiving on the other. If not using Bluetooth, select the operating system of the other device. Click the "Start Transfer" button on each device. On the sending device, select the files or folder to send. On the receiving device, select the folder in which to receive files. (To send a folder, check "Send Folder" before clicking "Start Transfer". A folder you send is recreated inside the destination folder on the receiving device, with its contents inside.)
    
    If using Bluetooth, confirm the 6-digit PIN on each side. The WiFi connection will be configured automatically. If not using Bluetooth, you will need to scan a QR code or type in a password.
    
    When prompted to join a WiFi network or modify WiFi settings, say Allow. On Windows you may have to grant permission to add a firewall rule. On macOS you may have to grant location permissions, which Apple requires to scan for WiFi networks. Flying Carpet does not read or collect your location, nor any other data.
    
    TROUBLESHOOTING

    Disable any VPN on both devices.

    If using Bluetooth fails, try manually unpairing the devices from one another and starting a new transfer.
    
    If sending from macOS to Linux, disable Bluetooth on both sides.

    Flying Carpet may make multiple attempts to join the other device's hotspot.
    
    Licensed under the GPL3: https://www.gnu.org/licenses/gpl-3.0.html#license-text`
"""
