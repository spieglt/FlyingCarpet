package dev.spiegl.flyingcarpet

import androidx.documentfile.provider.DocumentFile
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.InputStream
import java.nio.ByteBuffer
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

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
        encryptAndSendChunk(buffer.sliceArray(0 until bytesRead))
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

private fun MainViewModel.encryptAndSendChunk(chunk: ByteArray) {
    val secureRandom = SecureRandom()
    val nonce = ByteArray(96 / 8) // 96 bits
    secureRandom.nextBytes(nonce)
    var nonceString = ""
    for (byte in nonce) {
        nonceString += "%02x ".format(byte)
    }
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    val keySpec = SecretKeySpec(key, "AES")
    val gcmSpec = GCMParameterSpec(128, nonce)
    cipher.init(Cipher.ENCRYPT_MODE, keySpec, gcmSpec)

    // want to use update() here, but it doesn't let you change the nonce for each encryption, so this is what rust and swift do?
    val ciphertext = cipher.doFinal(chunk)
    val encryptedChunk = nonce + ciphertext

    // send size
    val size = longToBigEndianBytes(encryptedChunk.size.toLong())
    outputStream.write(size)
    // write chunk
    outputStream.write(encryptedChunk)
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
        val localHash = hashFile(file)
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
