package dev.spiegl.flyingcarpet

import android.app.Application
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import androidx.documentfile.provider.DocumentFile
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import java.io.File
import java.nio.ByteBuffer
import java.security.MessageDigest

fun getQrCodeBitmap(ssid: String, password: String): Bitmap {
    val size = 1024 // pixels
    val qrCodeContent = "$ssid;$password"
    val bits = QRCodeWriter().encode(qrCodeContent, BarcodeFormat.QR_CODE, size, size)
    return Bitmap.createBitmap(size, size, Bitmap.Config.RGB_565).also {
        for (x in 0 until size) {
            for (y in 0 until size) {
                it.setPixel(x, y, if (bits[x, y]) Color.BLACK else Color.WHITE)
            }
        }
    }
}

fun longToBigEndianBytes(n: Long): ByteArray {
    val byteBuffer = ByteBuffer.allocate(8)
    byteBuffer.putLong(n)
    byteBuffer.rewind()
    val byteArray = ByteArray(8)
    byteBuffer.get(byteArray)
    return byteArray
}

fun makeSizeReadable(context: Context, size: Long): String {
    val n = size.toDouble()
    return when {
        n < 1_000 -> context.getString(R.string.size_bytes, n)
        n < 1_000_000 -> context.getString(R.string.size_kb, n / 1_000)
        n < 1_000_000_000 -> context.getString(R.string.size_mb, n / 1_000_000)
        else -> context.getString(R.string.size_gb, n / 1_000_000_000)
    }
}

fun formatTime(context: Context, seconds: Double): String {
    return if (seconds > 60) {
        val minutes = seconds.toInt() / 60
        val remainder = seconds % 60
        if (minutes > 1) {
            context.getString(R.string.time_minutes_seconds, minutes, remainder)
        } else {
            context.getString(R.string.time_minute_seconds, minutes, remainder)
        }
    } else {
        context.getString(R.string.time_seconds, seconds)
    }
}

fun MainViewModel.makeParentDirectories(filename: String): DocumentFile? {
    var currentDir = DocumentFile.fromTreeUri(getApplication(), receiveDir)
    val childDirs = File(filename).parent?.split('/') ?: return null
    for (dir in childDirs) {
        if (currentDir == null) {
            throw Exception("Could not make parent directories, couldn't get currentDir.")
        }
        val proposedDir = currentDir.findFile(dir)
        currentDir = proposedDir ?: currentDir.createDirectory(dir)
    }
    return currentDir
}

// returns an array of tuples where the first item is the file and the second item is the path
// to get to it relative to root directory we're sending from
fun getFilesInDir(dir: DocumentFile, pathSoFar: String): Array<Pair<DocumentFile, String>> {
    var allFiles: Array<Pair<DocumentFile, String>> = arrayOf()
    val files = dir.listFiles()
    for (file in files) {
        if (file.isFile) {
            allFiles += file to pathSoFar
        } else if (file.isDirectory) {
            val newDirectoryPath = pathSoFar + '/' + file.name
            allFiles += getFilesInDir(file, newDirectoryPath)
        }
    }
    return allFiles
}

fun MainViewModel.hashFile(file: DocumentFile): ByteArray {
    val uri = file.uri
    val stream = getApplication<Application>().contentResolver.openInputStream(uri)
        ?: throw Exception("Could not open file to hash")
    val buffer = ByteArray(1_000_000)
    val hasher = MessageDigest.getInstance("SHA-256")
    do {
        val bytesRead = stream.read(buffer)
        hasher.update(buffer.sliceArray(IntRange(0, bytesRead - 1)))
    } while (bytesRead != -1)
    stream.close()
    return hasher.digest()
}

fun getSsidAndKey(password: String): Pair<String, ByteArray> {
    val hasher = MessageDigest.getInstance("SHA-256")
    hasher.update(password.encodeToByteArray())
    val key = hasher.digest()
    val ssid = "flyingCarpet_%02x%02x".format(key[0], key[1])
    return Pair(ssid, key)
}
