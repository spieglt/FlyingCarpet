package dev.spiegl.flyingcarpet

import android.app.Application
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

fun makeSizeReadable(size: Long): String {
    val n = size.toDouble()
    return when {
        n < 1_000 -> "$n bytes"
        n < 1_000_000 -> "%.2fKB".format(n / 1_000)
        n < 1_000_000_000 -> "%.2fMB".format(n / 1_000_000)
        else -> "%.2fGB".format(n / 1_000_000_000)
    }
}

fun formatTime(seconds: Double): String {
    return if (seconds > 60) {
        val minutes = seconds.toInt() / 60
        val remainder = seconds % 60
        if (minutes > 1) {
            "%d minutes %.2f seconds".format(minutes, remainder)
        } else {
            "%d minute %.2f seconds".format(minutes, remainder)
        }
    } else {
        "%.2f seconds".format(seconds)
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
