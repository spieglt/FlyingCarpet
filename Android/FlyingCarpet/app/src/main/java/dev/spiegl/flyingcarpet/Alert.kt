package dev.spiegl.flyingcarpet

import android.app.AlertDialog
import android.app.Dialog
import android.os.Bundle
import androidx.fragment.app.DialogFragment

class Alert(private val ssid: String, private val password: String) : DialogFragment() {
    override fun onCreateDialog(savedInstanceState: Bundle?): Dialog {
        return activity?.let {
            val builder = AlertDialog.Builder(it)
                .setMessage(getString(R.string.macos_wifi_info, ssid, password))
                .setPositiveButton(getString(R.string.ok)) { _, _ ->
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
                .setTitle(getString(R.string.about_title))
                .setMessage(getString(R.string.about_message))
            builder.create()
        } ?: throw IllegalStateException("Activity cannot be null")
    }
}
