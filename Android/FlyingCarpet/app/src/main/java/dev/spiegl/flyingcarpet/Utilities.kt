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
import androidx.core.graphics.createBitmap
import androidx.core.graphics.set

fun getQrCodeBitmap(ssid: String, password: String): Bitmap {
    return getQrCodeBitmapFromContent("$ssid;$password")
}

// shared network mode QR codes contain just the password, matching the desktop version
fun getQrCodeBitmapFromContent(qrCodeContent: String): Bitmap {
    val size = 1024 // pixels
    val bits = QRCodeWriter().encode(qrCodeContent, BarcodeFormat.QR_CODE, size, size)
    return createBitmap(size, size, Bitmap.Config.RGB_565).also {
        for (x in 0 until size) {
            for (y in 0 until size) {
                it[x, y] = if (bits[x, y]) Color.BLACK else Color.WHITE
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

// Peer-supplied filenames may carry "/" separators for folder transfers, but must not
// be able to escape the receive directory: reject ".." components and collapse
// empty/"." ones. Mirrors the desktop and Apple implementations — we don't rely on the
// SAF provider rejecting ".." display names.
fun sanitizeRelativeFilename(filename: String): String {
    val components = filename.split('/').filter { it.isNotEmpty() && it != "." }
    if (components.isEmpty() || components.contains("..")) {
        throw Exception("Received invalid filename: $filename")
    }
    return components.joinToString("/")
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
// to get to it relative to root directory we're sending from.
//
// Callers seed pathSoFar with the selected folder's own name, so the peer recreates that
// folder inside their chosen destination instead of having its contents dumped loose into
// it (matching the desktop and Apple senders; see docs/send-folder-behavior.md). The join is
// guarded so a relative path can never begin with "/": seeding from "" used to produce
// "/sub/file.jpg" for anything below the top level, which the desktop receiver rejects
// outright as a rooted path.
fun getFilesInDir(dir: DocumentFile, pathSoFar: String): Array<Pair<DocumentFile, String>> {
    var allFiles: Array<Pair<DocumentFile, String>> = arrayOf()
    val files = dir.listFiles()
    for (file in files) {
        if (file.isFile) {
            allFiles += file to pathSoFar
        } else if (file.isDirectory) {
            val name = file.name ?: continue
            val newDirectoryPath = if (pathSoFar.isEmpty()) name else "$pathSoFar/$name"
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
    while (true) {
        val bytesRead = stream.read(buffer)
        if (bytesRead == -1) {
            break
        }
        hasher.update(buffer, 0, bytesRead)
    }
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

fun computeHmac(key: ByteArray, data: ByteArray): ByteArray {
    val mac = javax.crypto.Mac.getInstance("HmacSHA256")
    mac.init(javax.crypto.spec.SecretKeySpec(key, "HmacSHA256"))
    return mac.doFinal(data)
}

fun verifyHmac(key: ByteArray, data: ByteArray, expected: ByteArray): Boolean {
    val computed = computeHmac(key, data)
    return MessageDigest.isEqual(computed, expected)
}
