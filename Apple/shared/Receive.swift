//
//  Receive.swift
//  FlyingCarpet
//
//  Created by Theron on 6/25/22.
//

import CryptoKit
import NetworkExtension
#if os(iOS)
import UIKit
#endif

extension Transfer {

    func receiveFiles() async throws {
        self.delegate?.output(msg: "Receiving files")

        guard let tcp = self.tcp else {
            throw TransferError.TCPReadError
        }

        let numFilesBigEndian: Data
        do {
            numFilesBigEndian = try await tcp.receiveNBytes(n: 8)
        } catch {
            self.delegate?.output(msg: "Error receiving number of files: \(error)")
            throw TransferError.TCPReadError
        }
        let numFiles = networkToInt64(bytes: numFilesBigEndian)
        // a negative count would trap forming the range below, and an absurd count means
        // a corrupt/hostile stream; a real transfer never approaches this
        guard numFiles >= 0 && numFiles <= MAX_FILE_COUNT else {
            throw TransferError.MalformedTransferHeader("file count \(numFiles) out of range")
        }

        for i in 0 ..< numFiles {
            self.delegate?.output(msg: "==========\nReceiving file \(i+1) of \(numFiles)")
            var filename = ""
            var fileSize = 0
            let start = DispatchTime.now()

            // receive file details
            let safeURL: URL
            do {
                (filename, fileSize) = try await self.receiveFileDetails()
                self.delegate?.output(msg: "Filename: \(filename)\nSize: \(makeHumanReadableFileSize(size: fileSize))")
                // resolve the peer-supplied filename to a path guaranteed to be inside
                // the chosen receive folder, before it touches the filesystem
                guard let receiveDir = self.receiveDir else {
                    throw TransferError.NoFilename(msg: "no receive directory set")
                }
                safeURL = try safeDestinationURL(baseDir: receiveDir, filename: filename)
                // determine if transfer is necessary
                let needTransfer = try await checkForFileReceiving(fullPath: safeURL.path, peerSize: fileSize)
                if !needTransfer {
                    self.delegate?.output(msg: "The same file already exists at this location, skipping.")
                    continue // hashes matched, skip this file
                }
            } catch let error as TransferError {
                // don't remap a specific error (e.g. UnsafeFilename) into a generic read error
                self.delegate?.output(msg: "Error receiving file details: \(error)")
                throw error
            } catch {
                self.delegate?.output(msg: "Error receiving file details: \(error)")
                throw TransferError.TCPReadError
            }
            var bytesLeft = fileSize

            // open output file
            try makeParentDirectories(for: safeURL)
            var outFileURL = safeURL

            var counter = 1
            while FileManager.default.fileExists(atPath: outFileURL.path) {
                // a file already exists at this path (with a different hash, or we'd have
                // skipped above); receive it under a "(n) name" sibling instead of clobbering
                let justDirs = safeURL.deletingLastPathComponent()
                let justFile = safeURL.lastPathComponent
                let newFilename = "(\(counter)) \(justFile)"
                outFileURL = justDirs.appendingPathComponent(newFilename)
                counter += 1
            }
            let outHandle: FileHandle
            do {
                if !FileManager.default.createFile(atPath: outFileURL.path, contents: nil) {
                    self.delegate?.output(msg: "Error: could not create output file")
                    throw TransferError.FileError
                }
                outHandle = try FileHandle(forUpdating: outFileURL)
            } catch {
                self.delegate?.output(msg: "Error opening output file: \(error)")
                throw TransferError.FileError
            }

            self.delegate?.setProgress(0, animated: false, hidden: false)

            // receive file
            while true {

                // check if user cancelled transfer
                await Task.yield()
                if Task.isCancelled {
                    self.delegate?.output(msg: "Cancelled, exiting.")
                    return
                }

                // receive chunk
                let decryptedChunk = try await self.receiveChunk()
                if decryptedChunk.count == 0 {
                    self.delegate?.output(msg: "File received")
                    break
                }
                bytesLeft -= decryptedChunk.count
                
                // add to output file
                try outHandle.seekToEnd()
                try outHandle.write(contentsOf: decryptedChunk)

                // update progress bar
                let progress = 1 - (Float(bytesLeft) / Float(fileSize))
                self.delegate?.setProgress(progress, animated: true, hidden: false)
                
            }

            self.delegate?.setProgress(1, animated: true, hidden: false)

            // tell sending end we're finished
            try await tcp.write(data: ONE)

            // stats
            let finish = DispatchTime.now()
            let elapsedSeconds = Double(finish.uptimeNanoseconds - start.uptimeNanoseconds) / 1_000_000_000
            let speed = (Double(fileSize * 8) / 1_000_000) / elapsedSeconds
            self.delegate?.output(msg: "Receiving took \(formatTime(seconds: elapsedSeconds))")
            self.delegate?.output(msg: String(format: "Speed: %.2fmbps", speed))

            // wait for double confirmation
            // if we're on the last file, let the double confirmation receiving time out after 2 seconds
            // in case the sending end tears down its hotspot before we receive it
            let lastFile = i == numFiles - 1
            if !lastFile {
                let _ = try await tcp.receiveNBytes(n: 8)
            } else {
                self.confirmed.value = false
                Task.detached {
                    // Task.sleep, not sleep(): detached tasks still run on the cooperative pool
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    self.killIt()
                }
                do {
                    let _ = try await tcp.receiveNBytes(n: 8)
                    self.confirmed.value = true
                } catch {
                    self.delegate?.output(msg: "Didn't receive confirmation")
                }
            }
        }
        #if os(iOS)
        if let link = self.receiveDir?.absoluteString.dropFirst(4) {
            self.delegate?.output(msg: "shareddocuments" + link)
//            let full = "shareddocuments\(link)"
//            if let url = URL(string: full) {
//                DispatchQueue.main.async {
//                    UIApplication.shared.open(url)
//                }
//            }
        }
        #endif
        self.delegate?.output(msg: "==========\nTransfer complete\n")
    }

    // v10+: file contents are protected by the Noise transport (see Noise.swift), which wraps
    // the whole connection, so chunks arrive as raw bytes here — no application-level
    // decryption.
    func receiveChunk() async throws -> Data {
        guard let tcp = self.tcp else { throw TransferError.TCPReadError }

        // get chunk size
        let chunkSizeBytes: Data
        do {
            chunkSizeBytes = try await tcp.receiveNBytes(n: 8)
        } catch {
            self.delegate?.output(msg: "Error receiving chunk size: \(error)")
            throw TransferError.TCPReadError
        }
        let chunkSizeInt64 = networkToInt64(bytes: chunkSizeBytes)
        // 0 is the end-of-file sentinel; a larger-than-possible value (or negative) means a
        // corrupt/hostile stream, and must be rejected before allocating a receive buffer of
        // that size
        guard chunkSizeInt64 >= 0 && chunkSizeInt64 <= Int64(MAX_CHUNK_BYTES) else {
            throw TransferError.MalformedTransferHeader("chunk size \(chunkSizeInt64) out of range")
        }
        let chunkSize = Int(chunkSizeInt64)
        if chunkSize == 0 {
            return Data()
        }
        // receive chunk (raw bytes; the Noise transport already authenticated and decrypted it)
        do {
            return try await tcp.receiveNBytes(n: chunkSize)
        } catch {
            self.delegate?.output(msg: "Error receiving chunk: \(error)")
            throw TransferError.TCPReadError
        }
    }

    func receiveFileDetails() async throws -> (filename: String, fileSize: Int) {
        guard let tcp = self.tcp else { throw TransferError.TCPReadError }

        // receive size of filename
        let filenameLenBytes: Data
        do {
            filenameLenBytes = try await tcp.receiveNBytes(n: 8)
        } catch {
            self.delegate?.output(msg: "Error receiving filename length: \(error)")
            throw TransferError.TCPReadError
        }
        let filenameLenInt64 = networkToInt64(bytes: filenameLenBytes)
        // reject a negative or absurd length before reading that many bytes: real paths
        // fit comfortably under this, and an unbounded value is a memory-exhaustion lever
        guard filenameLenInt64 >= 0 && filenameLenInt64 <= Int64(MAX_FILENAME_BYTES) else {
            throw TransferError.MalformedTransferHeader("filename length \(filenameLenInt64) out of range")
        }
        let filenameLen = Int(filenameLenInt64)

        // receive filename
        let filenameBytes: Data
        do {
            filenameBytes = try await tcp.receiveNBytes(n: filenameLen)
        } catch {
            self.delegate?.output(msg: "Error receiving filename: \(error)")
            throw TransferError.TCPReadError
        }
        guard let filename = String.init(data: filenameBytes, encoding: String.Encoding.utf8) else {
            throw TransferError.NoFilename(msg: "filename could not be converted to utf8")
        }

        // receive file size
        let fileSizeBytes: Data
        do {
            fileSizeBytes = try await tcp.receiveNBytes(n: 8)
        } catch {
            self.delegate?.output(msg: "Error receiving file size: \(error)")
            throw TransferError.TCPReadError
        }
        let fileSizeInt64 = networkToInt64(bytes: fileSizeBytes)
        let fileSize = Int.init(truncatingIfNeeded: fileSizeInt64)

        return (filename, fileSize)
    }

    func checkForFileReceiving(fullPath: String, peerSize: Int) async throws -> Bool {
        guard let tcp = self.tcp else { throw TransferError.TCPReadError }

        if FileManager.default.fileExists(atPath: fullPath) {
            // check for size
            let localSize = try getFileSize(file: URL.init(filePath: fullPath))
            if localSize != peerSize {
                try await tcp.write(data: ZERO)
                return true
            }
            // ask for hash, then calculate hash. if hash matches, reply that we don't need the file and continue.
            // if not, reply that we need the file and proceed.
            try await tcp.write(data: ONE)
            guard let localFile = FileHandle(forReadingAtPath: fullPath) else {
                throw TransferError.FileError
            }

            let localHash = try hashFile(file: localFile)
            let peerHash = try await tcp.receiveNBytes(n: 32)
            var hashesMatch = true
            localHash.withUnsafeBytes { bytes in
                for i in 0 ..< bytes.count {
                    if bytes[i] != peerHash[i] {
                        hashesMatch = false
                    }
                }
            }
            if hashesMatch { // one == hashes match, have file, don't need transfer. zero == hashes don't match, don't have file, need transfer.
                try await tcp.write(data: ONE)
            } else {
                try await tcp.write(data: ZERO)
            }
            return !hashesMatch
        } else {
            // tell sending end we don't have the file, don't need the hash, and can proceed to the transfer
            try await tcp.write(data: ZERO)
            return true
        }
    }
}
