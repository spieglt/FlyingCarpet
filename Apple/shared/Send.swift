//
//  Send.swift
//  FlyingCarpet
//
//  Created by Theron on 6/25/22.
//

import Foundation
import Network
import CryptoKit

extension Transfer {

    func sendFiles() async throws {
        self.delegate?.output(msg: "Sending files")

        guard let tcp = self.tcp else {
            throw TransferError.TCPReadError
        }

        // write number of files we're sending
        let numFiles = self.fileList.count
        let numFilesBytes = intToBigEndianBytes(n: numFiles)
        try await tcp.write(data: numFilesBytes)

        // send files
        for (i, file) in self.fileList.enumerated() {
            if numFiles > 1 {
                self.delegate?.output(msg: "==========\nBeginning transfer \(i+1) of \(numFiles).")
            }
            let start = DispatchTime.now()

            // send file details
            let fileSize = try await sendFileDetails(file: file)

            // open file handle
            let fileHandle = try FileHandle.init(forReadingFrom: file)

            // determine if transfer is necessary
            let needTransfer = try await checkForFileSending(fileHandle: fileHandle)
            if !needTransfer {
                self.delegate?.output(msg: "Recipient already has this file, skipping.")
                continue // hashes matched, skip this file
            }

            self.delegate?.setProgress(0, animated: false, hidden: false)


            // send file
            var bytesRead = 0
            while bytesRead < fileSize {
                // check if user cancelled transfer
                await Task.yield()
                if Task.isCancelled {
                    self.delegate?.output(msg: "Cancelled, exiting.")
                    return
                }

                // read data from file
                let currentChunk: Data
                do {
                    currentChunk = try fileHandle.read(upToCount: CHUNK_SIZE) ?? Data()
                } catch {
                    self.delegate?.output(msg: "Could not read from \(file). Error: \(error)")
                    throw error
                }
                // nil or empty means EOF. We told the receiver to expect fileSize bytes and
                // can't produce them, so fail the transfer instead of trapping on a nil unwrap.
                if currentChunk.isEmpty {
                    self.delegate?.output(msg: "Could not read from \(file): file ended after \(bytesRead) of \(fileSize) bytes.")
                    throw TransferError.FileError
                }
                bytesRead += currentChunk.count

                // send data (raw; the Noise transport encrypts the whole connection)
                try await self.sendChunk(chunk: currentChunk)

                // update progress bar
                let progress = Float(bytesRead) / Float(fileSize)
                self.delegate?.setProgress(progress, animated: true, hidden: false)
            }

            // we've written the whole file, send chunkSize of 0
            try await tcp.write(data: ZERO)

            // wait for receiving end to tell us they have everything
            let _ = try await tcp.receiveNBytes(n: 8)

            // stats
            self.delegate?.setProgress(1, animated: true, hidden: false)
            let finish = DispatchTime.now()
            let elapsedSeconds = Double(finish.uptimeNanoseconds - start.uptimeNanoseconds) / 1_000_000_000
            let speed = (Double(fileSize * 8) / 1_000_000) / elapsedSeconds
            self.delegate?.output(msg: "Sending took \(formatTime(seconds: elapsedSeconds))")
            self.delegate?.output(msg: String(format: "Speed: %.2fmbps", speed))

            // send double confirmation
            try await tcp.write(data: ONE)
        }
        self.delegate?.output(msg: "==========\nTransfer complete\n")
    }

    // v10+: file contents are protected by the Noise transport (see Noise.swift), which wraps
    // the whole connection, so chunks are sent as raw bytes here — no application-level
    // encryption.
    func sendChunk(chunk: Data) async throws {
        guard let tcp = self.tcp else { throw TransferError.TCPReadError }
        // length-prefixed raw bytes
        try await tcp.write(data: intToBigEndianBytes(n: chunk.count))
        try await tcp.write(data: chunk)
    }

    func sendFileDetails(file: URL) async throws -> Int {
        guard let tcp = self.tcp else { throw TransferError.TCPReadError }

        // send size of filename
        var filename = ""
        if self.sendFolder {
            var f = file.path
            f.trimPrefix(self.sendDir!.path)
            f.trimPrefix("/")
            filename = f
        } else {
            filename = file.lastPathComponent
        }
        let filenameBytes = Data(filename.utf8)
        try await tcp.write(data: intToBigEndianBytes(n: filenameBytes.count))

        // send filename
        try await tcp.write(data: filenameBytes)

        // send file size
        let fileSize = try getFileSize(file: file)
        try await tcp.write(data: intToBigEndianBytes(n: fileSize))

        self.delegate?.output(msg: "Filename: \(file.lastPathComponent)\nSize: \(makeHumanReadableFileSize(size: fileSize))")
        return fileSize
    }

    func checkForFileSending(fileHandle: FileHandle) async throws -> Bool {
        guard let tcp = self.tcp else { throw TransferError.TCPReadError }

        // we've sent the file details already, so need to wait for receiving end to tell us if they
        // have a file by that name and size. if so, hash and send. if not, proceed with transfer.
        let hasFileBytes = try await tcp.receiveNBytes(n: 8)
        let hasFile = networkToInt64(bytes: hasFileBytes) == 1
        if hasFile {
            // hash and send
            let localHash = try hashFile(file: fileHandle)
            var d = Data()
            localHash.withUnsafeBytes { bytes in
                d = Data(bytes)
            }
            try await tcp.write(data: Data(d))

            // if receiving end's copy of the file doesn't match, we need to do the transfer, so we return true
            // if they do match, we return false to indicate that we don't need to do the transfer
            let hashesMatchBytes = try await tcp.receiveNBytes(n: 8)
            let hashesMatch = networkToInt64(bytes: hashesMatchBytes) == 1
            return !hashesMatch
        } else {
            return true // receiving end doesn't have file, transfer is necessary
        }
    }
}
