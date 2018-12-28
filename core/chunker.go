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

func send(conn net.Conn, t *Transfer, fileNum int, ui UI) error {
	// setup
	start := time.Now()

	file, err := os.Open(t.FileList[fileNum])
	if err != nil {
		return errors.New("Error opening output file. Please quit and restart Flying Carpet.")
	}
	defer file.Close()

	ui.ShowProgressBar()
	fileSize, err := getSize(file)
	if err != nil {
		return errors.New("Could not read file size")
	}
	hash, err := getHash(t.FileList[fileNum])
	if err != nil {
		return err
	}
	ui.Output(fmt.Sprintf("File size: %s\nMD5 hash: %x", makeSizeReadable(fileSize), hash))
	numChunks := ceil(fileSize, CHUNKSIZE)

	bytesLeft := fileSize
	var i int64

	ticker := time.NewTicker(time.Millisecond * 1000)
	ticker.Stop()
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

	// transmit filename and size
	filename := filepath.Base(t.FileList[fileNum])
	filenameLen := int64(len(filename))
	err = binary.Write(conn, binary.BigEndian, filenameLen)
	if err != nil {
		return fmt.Errorf("Error writing filename length: %s\n Please quit and restart Flying Carpet.", err)
	}
	_, err = conn.Write([]byte(filename))
	if err != nil {
		return fmt.Errorf("Error writing filename: %s\n Please quit and restart Flying Carpet.", err)
	}
	err = binary.Write(conn, binary.BigEndian, fileSize)
	if err != nil {
		return fmt.Errorf("Error transmitting file size: %s\n Please quit and restart Flying Carpet.", err)
	}

	// send file
	buffer := make([]byte, CHUNKSIZE)
	var bytesRead int
	for i = 0; i < numChunks; i++ {
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting chunkAndSend, transfer was canceled.")
		default:
		}
		bytesRead, err = file.Read(buffer)
		if err == io.EOF {
			break
		}
		if err != nil {
			return fmt.Errorf("Error reading file: %s", err)
		}

		// encrypt buffer
		encryptedBuffer, err := encrypt(buffer[:bytesRead], t.Password)
		if err != nil {
			return err
		}

		// send size
		chunkSize := int64(len(encryptedBuffer))
		err = binary.Write(conn, binary.BigEndian, chunkSize)
		if err != nil {
			return errors.New("Error writing chunk length. Please quit and restart Flying Carpet. " + err.Error())
		}

		// send buffer
		bytes, err := conn.Write(encryptedBuffer)
		if bytes != len(encryptedBuffer) {
			return errors.New("Send error. Please quit and restart Flying Carpet. " + err.Error())
		}
	}
	// send chunkSize of 0 and then wait until receiving end tells us they have everything.
	binary.Write(conn, binary.BigEndian, int64(0))
	replyChan := make(chan int64)
	go func() {
		var comp int64
		binary.Read(conn, binary.BigEndian, &comp)
		replyChan <- comp
	}()
	select {
	case <-time.After(time.Second * 2):
		ui.Output("Receiving end did not acknowledge but should have received signal to close connection.")
	case /*comp :=*/ <-replyChan:
		// ui.Output(fmt.Sprintf("Receiving end says: %d", comp))
	}

	ui.UpdateProgressBar(100)
	ui.Output(fmt.Sprintf("Sending took %s", time.Since(start)))
	return nil
}

func receive(conn net.Conn, t *Transfer, fileNum int, ui UI) error {
	start := time.Now()

	// receive filename and size
	var filenameLen int64
	err := binary.Read(conn, binary.BigEndian, &filenameLen)
	if err != nil {
		return fmt.Errorf("Error receiving filename length: %s\nPlease quit and restart Flying Carpet.", err)
	}
	filenameBytes := make([]byte, filenameLen)
	_, err = io.ReadFull(conn, filenameBytes)
	if err != nil {
		return fmt.Errorf("Error receiving filename: %s\nPlease quit and restart Flying Carpet.", err)
	}
	filename := string(filenameBytes)
	var fileSize int64
	err = binary.Read(conn, binary.BigEndian, &fileSize)
	if err != nil {
		return fmt.Errorf("Error receiving file size: %s\nPlease quit and restart Flying Carpet.", err)
	}

	// check destination folder
	fpStat, err := os.Stat(t.ReceiveDir)
	if err != nil {
		return errors.New("Error accessing destination folder: " + err.Error())
	}
	if !fpStat.IsDir() {
		t.ReceiveDir = filepath.Dir(t.FileList[fileNum]) + string(os.PathSeparator)
	}

	// now check if file being received already exists. if so, find new filename.
	var currentFilePath string
	if _, err := os.Stat(t.ReceiveDir + filename); err != nil {
		t.FileList[fileNum] += filename
	} else {
		i := 0
		for _, err := os.Stat(t.ReceiveDir + fmt.Sprintf("%d_", i) + filename); err == nil; i++ {
		}
		currentFilePath = t.ReceiveDir + fmt.Sprintf("%d_", i) + filename
	}

	ui.Output(fmt.Sprintf("Filename: %s\nFile size: %s", filename, makeSizeReadable(fileSize)))
	// updateFilename(t)
	ui.ShowProgressBar()

	bytesLeft := fileSize

	// start ticker
	ticker := time.NewTicker(time.Millisecond * 1000)
	defer ticker.Stop()
	go func() {
		for _ = range ticker.C {
			select {
			case <-t.Ctx.Done():
				return
			default:
				percentDone := 100 * float64(float64(fileSize)-float64(bytesLeft)) / float64(fileSize)
				ui.UpdateProgressBar(int(percentDone))
			}
		}
	}()

	// receive file
	outFile, err := os.OpenFile(currentFilePath, os.O_CREATE|os.O_RDWR, 0666)
	if err != nil {
		return errors.New("Error creating out file. Please quit and restart Flying Carpet.")
	}
	defer outFile.Close()

	var chunkSize int64

	for {
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting dialPeer, transfer was canceled.")
		default:
		}
		// get chunk size
		chunkSize = -1
		err := binary.Read(conn, binary.BigEndian, &chunkSize)
		if err != nil || chunkSize == -1 {
			return errors.New("Error reading chunk size: " + err.Error())
		}
		if chunkSize == 0 {
			// done receiving
			break
		}

		// get chunk
		chunk := make([]byte, chunkSize)
		bytesReceived, err := io.ReadFull(conn, chunk)
		if err != nil {
			return errors.New("Error reading from stream: " + err.Error())
		}
		// ui.Output(fmt.Sprintf("read %d bytes", bytesReceived))
		if int64(bytesReceived) != chunkSize {
			return fmt.Errorf("bytesReceived: %d\nchunkSize: %d", bytesReceived, chunkSize)
		}

		// decrypt and add to outfile
		decryptedChunk, err := decrypt(chunk, t.Password)
		if err != nil {
			return err
		}
		_, err = outFile.Write(decryptedChunk)
		if err != nil {
			return errors.New("Error writing to out file. Please quit and restart Flying Carpet.")
		}
		bytesLeft -= int64(len(decryptedChunk))
	}

	// wait till we've received everything before signalling to other end that it's okay to stop sending.
	binary.Write(conn, binary.BigEndian, int64(1))

	ui.UpdateProgressBar(100)
	outFileSize, err := getSize(outFile)
	if err != nil {
		return errors.New("Could not read file size")
	}
	hash, err := getHash(currentFilePath)
	if err != nil {
		return err
	}
	ui.Output(fmt.Sprintf("Received file size: %s", makeSizeReadable(outFileSize)))
	ui.Output(fmt.Sprintf("Received file hash: %x", hash))
	ui.Output(fmt.Sprintf("Receiving took %s", time.Since(start)))

	speed := (float64(outFileSize*8) / 1000000) / (float64(time.Since(start)) / 1000000000)
	ui.Output(fmt.Sprintf("Speed: %.2fmbps", speed))
	return err
}

func sendCount(conn net.Conn, count int) error {
	numFiles := int64(count)
	err := binary.Write(conn, binary.BigEndian, numFiles)
	if err != nil {
		return fmt.Errorf("Error transmitting number of files: %s\n Please quit and restart Flying Carpet.", err)
	}
	return err
}

func receiveCount(conn net.Conn) (int, error) {
	var numFiles int64
	err := binary.Read(conn, binary.BigEndian, &numFiles)
	if err != nil {
		return 0, fmt.Errorf("Error receiving number of files: %s\nPlease quit and restart Flying Carpet.", err)
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
