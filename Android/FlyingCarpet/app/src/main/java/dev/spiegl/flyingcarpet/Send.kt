package dev.spiegl.flyingcarpet

import androidx.documentfile.provider.DocumentFile
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.InputStream
import java.nio.ByteBuffer

// v10+: file contents are protected by the Noise transport (see Noise.kt), which wraps the
// whole connection, so chunks are sent as raw bytes here — no application-level encryption.
suspend fun MainViewModel.sendFile(file: DocumentFile, fileStream: InputStream, filePath: String) {
    val start = System.currentTimeMillis()
    outputText("File size: ${makeSizeReadable(file.length())}")
    sendFileDetails(file, filePath)
    val needTransfer = checkForFileSending(file)
    if (!needTransfer) {
        outputText("Recipient already has this file, skipping.")
        return
    }
    var bytesLeft = file.length()
    val buffer = ByteArray(chunkSize)
    while (bytesLeft > 0) {
        val bytesRead = withContext(Dispatchers.IO) {
            fileStream.read(buffer)
        }
        if (bytesRead == -1) {
            outputText("Hit EOF, shouldn't have.")
            break
        }

        bytesLeft -= bytesRead
        sendChunk(buffer.sliceArray(0 until bytesRead))
        val percentDone = ((file.length() - bytesLeft).toDouble() / file.length()) * 100
        progressBarMut.postValue(percentDone.toInt())
    }

    // send chunkSize of 0 to signal end of transfer
    withContext(Dispatchers.IO) {
        outputStream.write(zero)
    }
    progressBarMut.postValue(100)

    // listen for receiving end to confirm that they have everything
    readNBytes(8, inputStream)

    // stats
    progressBarMut.postValue(100)
    val end = System.currentTimeMillis()
    val seconds = (end - start) / 1000.0
    outputText("Sending took ${formatTime(seconds)}")
    val megabits = 8 * (file.length() / 1_000_000.0)
    val mbps = megabits / seconds
    outputText("Speed: %.2fmbps".format(mbps))

    // write double confirmation
    withContext(Dispatchers.IO) {
        outputStream.write(one)
    }
}

private fun MainViewModel.sendChunk(chunk: ByteArray) {
    // length-prefixed raw bytes; confidentiality/integrity come from the Noise transport
    outputStream.write(longToBigEndianBytes(chunk.size.toLong()))
    outputStream.write(chunk)
}

private fun MainViewModel.sendFileDetails(file: DocumentFile, path: String) {
    // send size of filename
    if (file.name == null) {
        throw Exception("Could not get filename.")
    }
    val fullPath = path +
            if (path != "") { "/" } else { "" } +
            file.name!!
    val filenameBytes = fullPath.encodeToByteArray()
    val filenameSize = longToBigEndianBytes(filenameBytes.size.toLong())
    outputStream.write(filenameSize)
    // send filename
    outputStream.write(filenameBytes)
    // send file size
    outputStream.write(longToBigEndianBytes(file.length()))
}

private fun MainViewModel.checkForFileSending(file: DocumentFile): Boolean {
    // we've sent the file details already, so need to wait for receiving end to tell us if they
    // have a file by that name and size. if so, hash and send. if not, proceed with transfer.
    val hasFileBytes = readNBytes(8, inputStream)
    val hasFile = ByteBuffer.wrap(hasFileBytes).long == 1L
    return if (hasFile) {
        val localHash = hashFile(file.uri)
        outputStream.write(localHash)

        // if receiving end's copy of the file doesn't match, we need to do the transfer, so we return true
        // if they do match, we return false to indicate that we don't need to do the transfer
        val hashesMatchBytes = readNBytes(8, inputStream)
        val hashesMatch = ByteBuffer.wrap(hashesMatchBytes).long == 1L
        !hashesMatch
    } else {
        true
    }
}
