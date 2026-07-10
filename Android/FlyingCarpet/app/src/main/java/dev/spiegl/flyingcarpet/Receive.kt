package dev.spiegl.flyingcarpet

import androidx.documentfile.provider.DocumentFile
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer

// Sanity bounds on peer-supplied header values, checked before they're used to size an
// allocation. Chosen so every legitimate transfer passes while negative or absurd
// values (memory-exhaustion levers) are rejected as corrupt/hostile.
const val MAX_FILENAME_BYTES = 8192L
const val MAX_CHUNK_BYTES = chunkSize.toLong() // max raw chunk (Apple/Android use 5MB)

// v10+: file contents are protected by the Noise transport (see Noise.kt), which wraps the
// whole connection, so chunks arrive as raw bytes here — no application-level decryption.
suspend fun MainViewModel.receiveFile(lastFile: Boolean) {
    val start = System.currentTimeMillis()

    // receive file details. the filename is peer-supplied: allow "/" separators for
    // folder transfers, but nothing that could escape the receive directory.
    val (rawFilename, fileSize) = receiveFileDetails()
    val filename = sanitizeRelativeFilename(rawFilename)
    outputText("Filename: $filename.  Size: ${makeSizeReadable(fileSize)}")
    val needTransfer = checkForFileReceiving(filename, fileSize)
    if (!needTransfer) {
        outputText("The same file already exists at this location, skipping.")
        return
    }
    var bytesRead: Long = 0

    // detect if filename has folders in its path. if so, make them
    val destinationFolder = makeParentDirectories(filename)
        ?: DocumentFile.fromTreeUri(getApplication(), receiveDir)
        ?: throw Exception("Could not get DocumentFile from receiveDir.")

    // check if file being received already exists. if so, find new filename
    val newFilename =
        findNewFilename(destinationFolder, filename)

    // open output file
    val fileOutputStream = getOutputStreamForFile(destinationFolder, newFilename)

    // receive file
    while (true) {
        val chunk = receiveChunk()
        if (chunk.isEmpty()) {
            break
        }
        withContext(Dispatchers.IO) {
            fileOutputStream.write(chunk)
        }
        bytesRead += chunk.size
        val percentDone = (bytesRead.toDouble() / fileSize) * 100
        progressBarMut.postValue(percentDone.toInt())
    }

    // tell sending end we're finished
    withContext(Dispatchers.IO) {
        this@receiveFile.outputStream.write(one)
    }

    // stats
    progressBarMut.postValue(100)
    // outputText("Received $newFilename.")
    val end = System.currentTimeMillis()
    val seconds = (end - start) / 1000.0
    outputText("Receiving took ${formatTime(seconds)}")
    val megabits = 8 * (fileSize / 1_000_000.0)
    val mbps = megabits / seconds
    outputText("Speed: %.2fmbps".format(mbps))

    // wait for double confirmation
    // catch won't run in most cases because if peer closes hotspot, onLost in
    // NetworkCallback will fire, which will post transferFinished to MainActivity, which
    // will call cleanUpTransfer(), which will close inputStream and the coroutine this is
    // running in. but that's okay, i've tested it with sleep on sending end and it works.
    if (lastFile) {
        // timeout after 2 seconds in case sending end closes hotspot before we receive confirmation
        client.soTimeout = 2_000
        try {
            readNBytes(8, inputStream)
        } catch (e: Exception) {
            // swallowing this error because we don't want this to throw
            // if sending end tears stuff down and times out on last file
            outputText("Didn't receive confirmation from peer")
        }
    } else {
        // if not last file, we want to throw error if we can't read the confirmation
        readNBytes(8, inputStream)
    }
}

private fun MainViewModel.receiveChunk(): ByteArray {
    // receive size. 0 is the legitimate end-of-file sentinel; a larger-than-possible value
    // means a corrupt or hostile stream, and must be rejected before we allocate a receive
    // buffer of that size (or truncate it into a negative Int).
    val sizeBytes = readNBytes(8, inputStream)
    val size = ByteBuffer.wrap(sizeBytes).long

    if (size == 0L) {
        return ByteArray(0)
    }
    if (size < 0 || size > MAX_CHUNK_BYTES) {
        throw Exception("Chunk size $size from peer is out of range")
    }

    // receive chunk (raw bytes; the Noise transport already authenticated and decrypted it)
    return readNBytes(size.toInt(), inputStream)
}

private fun MainViewModel.receiveFileDetails(): Pair<String, Long> {
    // receive size of filename. real paths fit comfortably under the bound; a negative
    // or unbounded value is rejected before we allocate that many bytes.
    val filenameLenBytes = readNBytes(8, inputStream)
    val filenameLen = ByteBuffer.wrap(filenameLenBytes).long
    if (filenameLen < 0 || filenameLen > MAX_FILENAME_BYTES) {
        throw Exception("Filename length $filenameLen from peer is out of range")
    }

    // receive filename
    val filenameBytes = readNBytes(filenameLen.toInt(), inputStream)
    val filename = String(filenameBytes)

    // receive file size
    val fileSizeBytes = readNBytes(8, inputStream)
    val fileSize = ByteBuffer.wrap(fileSizeBytes).long

    return Pair(filename, fileSize)
}

// returns true if we need the transfer, false if not
private fun MainViewModel.checkForFileReceiving(filename: String, size: Long): Boolean {
    // check if file by this name and size exists
    var targetFile: DocumentFile? = DocumentFile.fromTreeUri(getApplication(), receiveDir)
        ?: throw Exception("Error reading folder: $receiveDir")
    val children = filename.split('/')
    var fileExists = true
    for (file in children) {
        targetFile = targetFile?.findFile(file)
        if (targetFile == null) {
            fileExists = false
            break
        }
    }
    if (fileExists) {
        // check size
        // currentDir should now be the actual file, so we can get its size
        if (targetFile != null && size == targetFile.length()) { // null check shouldn't be necessary since fileExists == true but cleaner than !!
            // name and size both match, so we need to ask sending end for the hash and calculate it ourselves
            outputStream.write(one)
            val localHash = hashFile(targetFile)
            val peerHash = readNBytes(32, inputStream)
            var hashesMatch = true
            for (i in 0 until 32) {
                if (localHash[i] != peerHash[i]) {
                    hashesMatch = false
                }
            }
            outputStream.write(if (hashesMatch) { one } else { zero })
            return !hashesMatch
        } else {
            outputStream.write(zero)
        }
    } else {
        // file doesn't exist so tell sending end we don't have it, we need the transfer, and return true
        outputStream.write(zero)
    }
    return true
}
