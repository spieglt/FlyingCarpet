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

const CHUNKSIZE = 1000000 // 1MB

func chunkAndSend(pConn *net.Conn, t *Transfer, fileNum int, ui UI) error {
	start := time.Now()
	conn := *pConn

	file, err := os.Open(t.FileList[fileNum])
	if err != nil {
		return errors.New("Error opening out file. Please quit and restart Flying Carpet.")
	}
	defer file.Close()

	// showProgressBar(t)
	fileSize := getSize(file)
	ui.Output(fmt.Sprintf("File size: %s\nMD5 hash: %x", makeSizeReadable(fileSize), getHash(t.FileList[fileNum])))
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
				// percentDone := 100 * float64(float64(fileSize)-float64(bytesLeft)) / float64(fileSize)
				// updateProgressBar(int(percentDone), t)
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
	/////////////////////////////

	for i = 0; i < numChunks; i++ {
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting chunkAndSend, transfer was canceled.")
		default:
			bufferSize := min(CHUNKSIZE, bytesLeft)
			buffer := make([]byte, bufferSize)
			bytesRead, err := file.Read(buffer)
			if int64(bytesRead) != bufferSize {
				return fmt.Errorf("bytesRead: %d\nbufferSize: %d\nError reading out file. Please quit and restart Flying Carpet.", bytesRead, bufferSize)
			}
			bytesLeft -= bufferSize

			// encrypt buffer
			encryptedBuffer := encrypt(buffer, t.Password)

			// send size of buffer
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
	}
	// send chunkSize of 0 and then wait until receiving end tells us they have everything.
	binary.Write(conn, binary.BigEndian, int64(0))

	// timeout for binary.Read
	replyChan := make(chan int64)
	timeoutChan := make(chan int)
	go func() {
		var comp int64
		binary.Read(conn, binary.BigEndian, &comp)
		replyChan <- comp
	}()
	go func() {
		time.Sleep(time.Second * 2)
		timeoutChan <- 0
	}()
	select {
	case /*comp :=*/ <-replyChan:
		// ui.Output(fmt.Sprintf("Receiving end says: %d", comp))
	case <-timeoutChan:
		ui.Output("Receiving end did not acknowledge but should have received signal to close connection.")
	}

	//////////

	// updateProgressBar(100, t)
	ui.Output(fmt.Sprintf("Sending took %s", time.Since(start)))
	return nil
}

func receiveAndAssemble(pConn *net.Conn, t *Transfer, fileNum int, ui UI) error {
	start := time.Now()
	conn := *pConn

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

	// if t.FileList[fileNum] is not a directory, we're in a multifile transfer (as start button action in gui.go
	// would've made it a directory for the first transfer), so we need to reset it to be a directory.
	fpStat, err := os.Stat(t.FileList[fileNum])
	if err != nil {
		return errors.New("Error accessing destination folder: " + err.Error())
	}
	if !fpStat.IsDir() {
		t.FileList[fileNum] = filepath.Dir(t.FileList[fileNum]) + string(os.PathSeparator)
	}

	// now check if file being received already exists. if so, append t.SSID to front end.
	if _, err := os.Stat(t.FileList[fileNum] + filename); err != nil {
		t.FileList[fileNum] += filename
	} else {
		t.FileList[fileNum] = t.FileList[fileNum] + t.SSID + "_" + filename
	}

	ui.Output(fmt.Sprintf("Filename: %s\nFile size: %s", filename, makeSizeReadable(fileSize)))
	// updateFilename(t)
	// progress bar
	// showProgressBar(t)
	bytesLeft := fileSize
	ticker := time.NewTicker(time.Millisecond * 1000)
	defer ticker.Stop()
	go func() {
		for _ = range ticker.C {
			select {
			case <-t.Ctx.Done():
				return
			default:
				// percentDone := 100 * float64(float64(fileSize)-float64(bytesLeft)) / float64(fileSize)
				// updateProgressBar(int(percentDone), t)
			}
		}
	}()
	/////////////////////////////

	outFile, err := os.OpenFile(t.FileList[fileNum], os.O_CREATE|os.O_RDWR, 0666)
	if err != nil {
		return errors.New("Error creating out file. Please quit and restart Flying Carpet.")
	}
	defer outFile.Close()

	var chunkSize int64
outer:
	for {
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting dialPeer, transfer was canceled.")
		default:
			// get chunk size
			chunkSize = -1
			err := binary.Read(conn, binary.BigEndian, &chunkSize)
			if err != nil || chunkSize == -1 {
				return errors.New("Error reading chunk size: " + err.Error())
			}
			if chunkSize == 0 {
				// done receiving
				break outer
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
			decryptedChunk := decrypt(chunk, t.Password)
			_, err = outFile.Write(decryptedChunk)
			if err != nil {
				return errors.New("Error writing to out file. Please quit and restart Flying Carpet.")
			}
			bytesLeft -= int64(len(decryptedChunk))
		}
	}

	// wait till we've received everything before signalling to other end that it's okay to stop sending.
	binary.Write(conn, binary.BigEndian, int64(1))

	// updateProgressBar(100, t)
	ui.Output(fmt.Sprintf("Received file size: %s", makeSizeReadable(getSize(outFile))))
	ui.Output(fmt.Sprintf("Received file hash: %x", getHash(t.FileList[fileNum])))
	ui.Output(fmt.Sprintf("Receiving took %s", time.Since(start)))

	speed := (float64(getSize(outFile)*8) / 1000000) / (float64(time.Since(start)) / 1000000000)
	ui.Output(fmt.Sprintf("Speed: %.2fmbps", speed))
	return err
}

func sendCount(pConn *net.Conn, t *Transfer) error {
	conn := *pConn
	numFiles := int64(len(t.FileList))
	err := binary.Write(conn, binary.BigEndian, numFiles)
	if err != nil {
		return fmt.Errorf("Error transmitting number of files: %s\n Please quit and restart Flying Carpet.", err)
	}
	return err
}

func receiveCount(pConn *net.Conn, t *Transfer) (int, error) {
	conn := *pConn
	var numFiles int64
	err := binary.Read(conn, binary.BigEndian, &numFiles)
	if err != nil {
		return 0, fmt.Errorf("Error receiving number of files: %s\nPlease quit and restart Flying Carpet.", err)
	}
	return int(numFiles), nil
}

func getSize(file *os.File) (size int64) {
	fileInfo, _ := file.Stat()
	size = fileInfo.Size()
	return
}

func getHash(filePath string) (md5hash []byte) {
	file, err := os.Open(filePath)
	if err != nil {
		panic(err)
	}
	hash := md5.New()
	if _, err := io.Copy(hash, file); err != nil {
		panic(err)
	}
	md5hash = hash.Sum(nil)
	return
}

// func updateProgressBar(percentage int, t *Transfer) {

// }

// func showProgressBar(t *Transfer) {

// }

// func updateFilename(t *Transfer) {

// }

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