package core

import (
	"crypto/md5"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"time"
)

// CHUNKSIZE is 1MB
const CHUNKSIZE = 1000000

// TCPTIMEOUT is 10 seconds
const TCPTIMEOUT = 10

// NUMRETRIES is 3
const NUMRETRIES = 3

type fileDetail struct {
	FileName string
	FileSize int
	Hash     []byte
}

// needed?
type chunkDetail struct {
	Size int
}

func sendFile(conn net.Conn, t *Transfer, fileNum int, ui UI) error {
	// setup
	start := time.Now()

	// open outgoing file
	file, err := os.Open(t.FileList[fileNum])
	if err != nil {
		return errors.New("Error opening output file")
	}
	defer file.Close()

	// get details
	fileSize, err := getSize(file)
	if err != nil {
		return errors.New("Could not read file size")
	}
	hash, err := getHash(t.FileList[fileNum])
	if err != nil {
		return err
	}
	ui.Output(fmt.Sprintf("File size: %s\nMD5 hash: %x", makeSizeReadable(fileSize), hash))
	bytesLeft := fileSize

	// set deadline for write
	extendDeadline(conn)

	// send file details
	sendFileDetails(
		conn,
		t.FileList[fileNum],
		fileSize,
		fmt.Sprintf("%x", hash))

	// show progress bar and start updating it
	ui.ShowProgressBar()
	ticker := time.NewTicker(time.Millisecond * 1000)
	defer ticker.Stop()
	go func() {
		for range ticker.C {
			select {
			case <-t.Ctx.Done():
				return
			default:
				percentDone := 100 * float64(float64(fileSize)-float64(bytesLeft)) / float64(fileSize)
				ui.UpdateProgressBar(int(percentDone))
			}
		}
	}()

	// send file
	buffer := make([]byte, CHUNKSIZE)
	for {
		// bail if user canceled transfer
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting send, transfer was canceled")
		default:
		}
		// fill the buffer with bytes
		bytesRead, err := file.Read(buffer)
		if err == io.EOF {
			break // done reading file
		}
		if err != nil {
			return fmt.Errorf("Error reading file: %s", err)
		}
		bytesLeft -= int64(bytesRead) // for ticker
		// try to send, retrying if there's a timeout
		for retry := 0; retry < NUMRETRIES; retry++ {
			extendDeadline(conn)
			err = encryptAndSendChunk(buffer[:bytesRead], t.Password, conn)
			if err != nil {
				switch errType := err.(type) {
				case net.Error:
					if errType.Timeout() {
						// if it timed out, retry
						ui.Output(fmt.Sprintf("Retrying %d more times", NUMRETRIES-retry))
						continue
					}
					return err
				default:
					return err
				}
			}
			break
		}
	}

	// send chunkSize of 0 and then wait until receiving end tells us they have everything.
	extendDeadline(conn)
	var comp int64 = -1
	err = binary.Write(conn, binary.BigEndian, int64(0))
	if err != nil {
		return err
	}
	err = binary.Read(conn, binary.BigEndian, &comp)
	if err != nil {
		return err
	}

	// print stats
	ui.UpdateProgressBar(100)
	ui.Output(fmt.Sprintf("Sending took %s", time.Since(start)))

	speed := (float64(fileSize*8) / 1000000) / (float64(time.Since(start)) / 1000000000)
	ui.Output(fmt.Sprintf("Speed: %.2fmbps", speed))
	return err
}

func encryptAndSendChunk(chunk []byte, pw string, conn net.Conn) (err error) {
	encryptedChunk, err := encrypt(chunk, pw)
	if err != nil {
		return err
	}
	// send size
	chunkSize := int64(len(encryptedChunk))
	err = binary.Write(conn, binary.BigEndian, chunkSize)
	if err != nil {
		return errors.New("Error writing chunk length: " + err.Error())
	}

	bytesWritten, err := conn.Write(encryptedChunk)
	if err != nil {
		return err
	}
	if bytesWritten != len(encryptedChunk) {
		return errors.New("Send error: not all bytes written")
	}
	return
}

func receiveFile(conn net.Conn, t *Transfer, fileNum int, ui UI) error {
	// setup
	start := time.Now()

	// set deadline for read
	extendDeadline(conn)

	// get file details
	fileName, fileSize, fileHash, err := receiveFileDetails(conn)
	if err != nil {
		return err
	}
	bytesLeft := fileSize

	// check destination folder
	fpStat, err := os.Stat(t.ReceiveDir)
	if err != nil {
		return errors.New("Error accessing destination folder: " + err.Error())
	}
	if !fpStat.IsDir() {
		t.ReceiveDir = filepath.Dir(t.FileList[fileNum]) + string(os.PathSeparator)
	}

	// now check if file being received already exists. if so, find new filename.
	// err == nil means file is there. err != nil means file is not there.
	var currentFilePath string
	if _, err := os.Stat(t.ReceiveDir + string(os.PathSeparator) + fileName); err == nil {
		i := 1
		for err == nil {
			_, err = os.Stat(t.ReceiveDir + string(os.PathSeparator) + fmt.Sprintf("%d_", i) + fileName)
			if err == nil {
				i++
			}
		}
		currentFilePath = t.ReceiveDir + string(os.PathSeparator) + fmt.Sprintf("%d_", i) + fileName
	} else {
		currentFilePath = t.ReceiveDir + string(os.PathSeparator) + fileName
	}

	ui.Output(fmt.Sprintf("Filename: %s\nFile size: %s", currentFilePath, makeSizeReadable(int64(fileSize))))

	// show progress bar and start updating it
	ui.ShowProgressBar()
	ticker := time.NewTicker(time.Millisecond * 1000)
	defer ticker.Stop()
	go func() {
		for range ticker.C {
			select {
			case <-t.Ctx.Done():
				return
			default:
				percentDone := 100 * float64(float64(fileSize)-float64(bytesLeft)) / float64(fileSize)
				ui.UpdateProgressBar(int(percentDone))
			}
		}
	}()

	// open output file
	outFile, err := os.OpenFile(currentFilePath, os.O_CREATE|os.O_RDWR, 0666)
	if err != nil {
		return errors.New("Error creating out file: " + err.Error())
	}
	defer outFile.Close()
outer:
	for {
		// bail if user canceled transfer
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting receive, transfer was canceled")
		default:
		}
		// try to receive, retrying if there's a timeout
		for retry := 0; retry < NUMRETRIES; retry++ {
			extendDeadline(conn)
			bytesDecrypted, err := receiveAndDecryptChunk(outFile, t.Password, conn)
			if bytesDecrypted == 0 {
				break outer
			}
			if err != nil {
				switch errType := err.(type) {
				case net.Error:
					if errType.Timeout() {
						// if it timed out, retry
						ui.Output(fmt.Sprintf("Retrying %d more times", NUMRETRIES-retry))
						continue
					}
					return err
				default:
					return err
				}
			}
			bytesLeft -= int64(bytesDecrypted)
		}
	}

	// tell sending end we're finished
	binary.Write(conn, binary.BigEndian, int64(1))

	// stats
	ui.UpdateProgressBar(100)
	outFileSize, err := getSize(outFile)
	if err != nil {
		return errors.New("Could not read file size")
	}
	hash, err := getHash(currentFilePath)
	if err != nil {
		return err
	}
	if fmt.Sprintf("%x", hash) != fileHash {
		return fmt.Errorf("Mismatched file hashes!\nHash sent at start of transfer: %x\nHash of received file: %x\nOutput size: %d",
			fileHash, hash, outFileSize)
	}
	ui.Output(fmt.Sprintf("Received file size: %s", makeSizeReadable(outFileSize)))
	ui.Output(fmt.Sprintf("Received file hash: %x", hash))
	ui.Output(fmt.Sprintf("Receiving took %s", time.Since(start)))

	speed := (float64(outFileSize*8) / 1000000) / (float64(time.Since(start)) / 1000000000)
	ui.Output(fmt.Sprintf("Speed: %.2fmbps", speed))
	return err
}

func receiveAndDecryptChunk(outFile *os.File, pw string, conn net.Conn) (bytesDecrypted int, err error) {
	// get chunk size
	var chunkSize int64 = -1
	err = binary.Read(conn, binary.BigEndian, &chunkSize)
	if err != nil || chunkSize == -1 {
		return 0, errors.New("Error reading chunk size: " + err.Error())
	}
	if chunkSize == 0 {
		return // done receiving
	}
	// receive chunk
	chunk := make([]byte, chunkSize)
	bytesReceived, err := io.ReadFull(conn, chunk)
	if err != nil {
		return 0, errors.New("Error reading from stream: " + err.Error())
	}
	if int64(bytesReceived) != chunkSize {
		return 0, fmt.Errorf("bytesReceived: %d\ndetail.Size: %d", bytesReceived, chunkSize)
	}
	// decrypt
	decryptedChunk, err := decrypt(chunk, pw)
	if err != nil {
		return
	}
	// add to output file
	_, err = outFile.Write(decryptedChunk)
	if err != nil {
		return
	}
	// return number of decrypted bytes for progress bar
	bytesDecrypted = len(decryptedChunk)
	return
}

func sendFileDetails(conn net.Conn, name string, size int64, hash string) (err error) {
	// send size of filename
	filenameLen := int64(len(name))
	err = binary.Write(conn, binary.BigEndian, filenameLen)
	if err != nil {
		return fmt.Errorf("Error sending filename length: %s", err)
	}
	// send filename
	_, err = conn.Write([]byte(name))
	if err != nil {
		return fmt.Errorf("Error sending filename: %s", err)
	}
	// send file size
	err = binary.Write(conn, binary.BigEndian, size)
	if err != nil {
		return fmt.Errorf("Error sending file size: %s", err)
	}
	// send size of file hash
	hashSize := int64(len(hash))
	err = binary.Write(conn, binary.BigEndian, hashSize)
	if err != nil {
		return fmt.Errorf("Error sending size of file hash: %s", err)
	}
	// send file hash
	_, err = conn.Write([]byte(hash))
	if err != nil {
		return fmt.Errorf("Error sending file hash: %s", err)
	}
	return
}

func receiveFileDetails(conn net.Conn) (name string, size int64, hash string, err error) {
	// receive size of filename
	var filenameLen int64
	err = binary.Read(conn, binary.BigEndian, &filenameLen)
	if err != nil {
		return "", 0, "", fmt.Errorf("Error receiving filename length: %s", err)
	}
	// receive filename
	filenameBytes := make([]byte, filenameLen)
	_, err = io.ReadFull(conn, filenameBytes)
	if err != nil {
		return "", 0, "", fmt.Errorf("Error receiving filename: %s", err)
	}
	name = string(filenameBytes)
	// receive file size
	err = binary.Read(conn, binary.BigEndian, &size)
	if err != nil {
		return "", 0, "", fmt.Errorf("Error receiving file size: %s", err)
	}
	// receive size of file hash
	var hashSize int64
	err = binary.Read(conn, binary.BigEndian, &hashSize)
	if err != nil {
		return "", 0, "", fmt.Errorf("Error receiving size of file hash: %s", err)
	}
	// receive file hash
	hashBytes := make([]byte, hashSize)
	_, err = io.ReadFull(conn, hashBytes)
	if err != nil {
		return "", 0, "", fmt.Errorf("Error receiving file hash: %s", err)
	}
	hash = string(hashBytes)
	return
}

func sendCount(conn net.Conn, count int) error {
	numFiles := int64(count)
	err := binary.Write(conn, binary.BigEndian, numFiles)
	if err != nil {
		return fmt.Errorf("Error transmitting number of files: %s", err)
	}
	return err
}

func receiveCount(conn net.Conn) (int, error) {
	var numFiles int64
	err := binary.Read(conn, binary.BigEndian, &numFiles)
	if err != nil {
		return 0, fmt.Errorf("Error receiving number of files: %s", err)
	}
	return int(numFiles), nil
}

func getSize(file *os.File) (size int64, err error) {
	fileInfo, err := file.Stat()
	if err != nil {
		return 0, err
	}
	size = fileInfo.Size()
	return
}

func getHash(filePath string) (md5hash []byte, err error) {
	file, err := os.Open(filePath)
	if err != nil {
		return nil, err
	}
	hash := md5.New()
	if _, err := io.Copy(hash, file); err != nil {
		return nil, err
	}
	md5hash = hash.Sum(nil)
	return
}

func ceil(x, y int64) int64 {
	if x%y != 0 {
		return ((x / y) + 1)
	}
	return x / y
}

func min(x, y int64) int64 {
	if x < y {
		return x
	}
	return y
}

func makeSizeReadable(size int64) string {
	v := float64(size)
	switch {
	case v < 1000:
		return fmt.Sprintf("%.0f bytes", v)
	case v < 1000000:
		return fmt.Sprintf("%.2fKB", v/1000)
	case v < 1000000000:
		return fmt.Sprintf("%.2fMB", v/1000000)
	default:
		return fmt.Sprintf("%.2fGB", v/1000000000)
	}
}

func extendDeadline(conn net.Conn) {
	conn.SetDeadline(time.Now().Add(time.Second * TCPTIMEOUT))
}
