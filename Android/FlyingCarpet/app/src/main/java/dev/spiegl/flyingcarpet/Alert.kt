package dev.spiegl.flyingcarpet

import android.app.AlertDialog
import android.app.Dialog
import android.os.Bundle
import androidx.fragment.app.DialogFragment

class Alert(private val ssid: String, private val password: String) : DialogFragment() {
    override fun onCreateDialog(savedInstanceState: Bundle?): Dialog {
        return activity?.let {
            val builder = AlertDialog.Builder(it)
//            builder.setTitle("Enter on Peer")
                .setMessage("Start the transfer on macOS and enter these when prompted:\n\nSSID: $ssid\nPassword: $password")
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
    Version 8.0.2
    theron@spiegl.dev
    Copyright 2024, Theron Spiegl, all rights reserved.
    
    Flying Carpet performs file transfers between Android, iOS, Linux, Mac, and Windows devices with wireless cards via ad hoc WiFi. No access point or cell connection is required.
"""
