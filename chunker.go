package main

import (
	"crypto/md5"
	"encoding/binary"
	"errors"
	"fmt"
	"github.com/dontpanic92/wxGo/wx"
	"io"
	"net"
	"os"
	"path/filepath"
	"time"
)

const CHUNKSIZE = 1000000 // 1MB

func chunkAndSend(pConn *net.Conn, t *Transfer) error {
	start := time.Now()
	var conn net.Conn = *pConn

	file, err := os.Open(t.Filepath)
	if err != nil {
		return errors.New("Error opening out file. Please quit and restart Flying Carpet.")
	}
	defer file.Close()

	showProgressBar(t)
	fileSize := getSize(file)
	t.output(fmt.Sprintf("File size: %s\nMD5 hash: %x", makeSizeReadable(fileSize), getHash(t.Filepath)))
	numChunks := ceil(fileSize, CHUNKSIZE)

	bytesLeft := fileSize
	var i int64

	ticker := time.NewTicker(time.Millisecond * 1000)
	go func() {
		for _ = range ticker.C {
			select {
			case <-t.Ctx.Done():
				return
			default:
				percentDone := 100 * float64(float64(fileSize)-float64(bytesLeft)) / float64(fileSize)
				updateProgressBar(int(percentDone), t)
			}
		}
	}()

	// transmit filename and size
	filename := filepath.Base(t.Filepath)
	filenameLen := int64(len(filename))
	err = binary.Write(conn, binary.BigEndian, filenameLen)
	if err != nil {
		return errors.New(fmt.Sprintf("Error writing filename length: %s\n Please quit and restart Flying Carpet.", err))
	}
	_, err = conn.Write([]byte(filename))
	if err != nil {
		return errors.New(fmt.Sprintf("Error writing filename: %s\n Please quit and restart Flying Carpet.", err))
	}
	err = binary.Write(conn, binary.BigEndian, fileSize)
	if err != nil {
		return errors.New(fmt.Sprintf("Error transmitting file size: %s\n Please quit and restart Flying Carpet.", err))
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
				return errors.New(fmt.Sprintf("bytesRead: %d\nbufferSize: %d\nError reading out file. Please quit and restart Flying Carpet.", bytesRead, bufferSize))
			}
			bytesLeft -= bufferSize

			// encrypt buffer
			encryptedBuffer := encrypt(buffer, t.Passphrase)

			// send size of buffer
			chunkSize := int64(len(encryptedBuffer))
			err = binary.Write(conn, binary.BigEndian, chunkSize)
			if err != nil {
				return errors.New("Error writing chunk length. Please quit and restart Flying Carpet.")
			}

			// send buffer
			bytes, err := conn.Write(encryptedBuffer)
			if bytes != len(encryptedBuffer) {
				return errors.New("Send error. Please quit and restart Flying Carpet.")
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
		// t.output(fmt.Sprintf("Receiving end says: %d", comp))
	case <-timeoutChan:
		t.output("Receiving end did not acknowledge but should have received signal to close connection.")
	}

	//////////

	ticker.Stop()
	updateProgressBar(100, t)
	t.output(fmt.Sprintf("Sending took %s", time.Since(start)))
	return nil
}

func receiveAndAssemble(pConn *net.Conn, t *Transfer) error {
	start := time.Now()
	var conn net.Conn = *pConn

	// receive filename and size
	var filenameLen int64
	err := binary.Read(conn, binary.BigEndian, &filenameLen)
	if err != nil {
		return errors.New(fmt.Sprintf("Error receiving filename length: %s\nPlease quit and restart Flying Carpet.", err))
	}
	filenameBytes := make([]byte, filenameLen)
	_, err = io.ReadFull(conn, filenameBytes)
	if err != nil {
		return errors.New(fmt.Sprintf("Error receiving filename: %s\nPlease quit and restart Flying Carpet.", err))
	}
	filename := string(filenameBytes)
	var fileSize int64
	err = binary.Read(conn, binary.BigEndian, &fileSize)
	if err != nil {
		return errors.New(fmt.Sprintf("Error receiving file size: %s\nPlease quit and restart Flying Carpet.", err))
	}

	// if t.Filepath is not a directory, we're in a multifile transfer (as start button action in gui.go
	// would've made it a directory for the first transfer), so we need to reset it to be a directory.
	fpStat, err := os.Stat(t.Filepath)
	if err != nil {
		return errors.New("Error accessing destination folder: " + err.Error())
	}
	if !fpStat.IsDir() {
		t.Filepath = filepath.Dir(t.Filepath) + string(os.PathSeparator)
	}

	// now check if file being received already exists. if so, append t.SSID to front end.
	if _, err := os.Stat(t.Filepath + filename); err != nil {
		t.Filepath += filename
	} else {
		t.Filepath = t.Filepath + t.SSID + "_" + filename
	}


	t.output(fmt.Sprintf("Filename: %s\nFile size: %s", filename, makeSizeReadable(fileSize)))
	updateFilename(t)
	// progress bar
	showProgressBar(t)
	bytesLeft := fileSize
	ticker := time.NewTicker(time.Millisecond * 1000)
	go func() {
		for _ = range ticker.C {
			select {
			case <-t.Ctx.Done():
				return
			default:
				percentDone := 100 * float64(float64(fileSize)-float64(bytesLeft)) / float64(fileSize)
				updateProgressBar(int(percentDone), t)
			}
		}
	}()
	/////////////////////////////

	outFile, err := os.OpenFile(t.Filepath, os.O_CREATE|os.O_RDWR, 0666)
	if err != nil {
		return errors.New("Error creating out file. Please quit and restart Flying Carpet.")
	}
	defer outFile.Close()
outer:
	for {
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting dialPeer, transfer was canceled.")
		default:
			// get chunk size
			var chunkSize int64
			err := binary.Read(conn, binary.BigEndian, &chunkSize)
			if err != nil {
				t.output(fmt.Sprintf("err: %s", err.Error()))
			}
			if chunkSize == 0 {
				// done receiving
				break outer
			}

			// get chunk
			chunk := make([]byte, chunkSize)
			bytesReceived, err := io.ReadFull(conn, chunk)
			if err != nil {
				t.output("Error reading from stream. Retrying.")
				t.output(err.Error())
				continue
			}
			// t.output(fmt.Sprintf("read %d bytes", bytesReceived))
			if int64(bytesReceived) != chunkSize {
				t.output(fmt.Sprintf("bytesReceived: %d\nchunkSize: %d", bytesReceived, chunkSize))
			}

			// decrypt and add to outfile
			decryptedChunk := decrypt(chunk, t.Passphrase)
			_, err = outFile.Write(decryptedChunk)
			if err != nil {
				return errors.New("Error writing to out file. Please quit and restart Flying Carpet.")
			}
			bytesLeft -= int64(len(decryptedChunk))
		}
	}

	// wait till we've received everything before signalling to other end that it's okay to stop sending.
	binary.Write(conn, binary.BigEndian, int64(1))

	ticker.Stop()
	updateProgressBar(100, t)
	t.output(fmt.Sprintf("Received file size: %d", getSize(outFile)))
	t.output(fmt.Sprintf("Received file hash: %x", getHash(t.Filepath)))
	t.output(fmt.Sprintf("Receiving took %s", time.Since(start)))

	speed := (float64(getSize(outFile)*8) / 1000000) / (float64(time.Since(start)) / 1000000000)
	t.output(fmt.Sprintf("Speed: %.2fmbps", speed))
	return err
}

func sendCount(pConn *net.Conn, t *Transfer) error {
	var conn net.Conn = *pConn
	numFiles := int64(len(t.FileList))
	err := binary.Write(conn, binary.BigEndian, numFiles)
	if err != nil {
		return errors.New(fmt.Sprintf("Error transmitting number of files: %s\n Please quit and restart Flying Carpet.", err))
	}
	return err
}

func receiveCount(pConn *net.Conn, t *Transfer) (int, error) {
	var conn net.Conn = *pConn
	var numFiles int64
	err := binary.Read(conn, binary.BigEndian, &numFiles)
	if err != nil {
		return 0, errors.New(fmt.Sprintf("Error receiving number of files: %s\nPlease quit and restart Flying Carpet.", err))
	}
	return int(numFiles), nil
}

func getSize(file *os.File) (size int64) {
	fileInfo, _ := file.Stat()
	size = fileInfo.Size()
	return
}

func getHash(filepath string) (md5hash []byte) {
	file, err := os.Open(filepath)
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

func updateProgressBar(percentage int, t *Transfer) {
	progressEvt := wx.NewThreadEvent(wx.EVT_THREAD, PROGRESS_BAR_UPDATE)
	progressEvt.SetInt(percentage)
	t.Frame.QueueEvent(progressEvt)
}

func showProgressBar(t *Transfer) {
	progressEvt := wx.NewThreadEvent(wx.EVT_THREAD, PROGRESS_BAR_SHOW)
	t.Frame.QueueEvent(progressEvt)
}

func updateFilename(t *Transfer) {
	filenameEvt := wx.NewThreadEvent(wx.EVT_THREAD, RECEIVE_FILE_UPDATE)
	filenameEvt.SetString(t.Filepath)
	t.Frame.QueueEvent(filenameEvt)
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
		return fmt.Sprintf("%.2fB",v)
	case v < 1000000:
		return fmt.Sprintf("%.2fKB",v/1000)
	case v < 1000000000:
		return fmt.Sprintf("%.2fMB",v/1000000)
	default:
		return fmt.Sprintf("%.2fGB",v/1000000000)
	}
}
