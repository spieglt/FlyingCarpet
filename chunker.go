package main

import (
	// "bufio"
	"crypto/md5"
	"encoding/binary"
	"fmt"
	"github.com/dontpanic92/wxGo/wx"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"time"
)

const CHUNKSIZE = 1000000 // 1MB

func (t *Transfer) chunkAndSend(sendChan chan bool, n Network) {

	start := time.Now()
	defer t.Conn.Close()

	file, err := os.Open(t.Filepath)
	if err != nil {
		n.teardown(t)
		t.output("Error opening out file. Please quit and restart Flying Carpet.")
		sendChan <- false
		return
	}
	defer file.Close()

	t.showProgressBar()
	fileSize := getSize(file)
	t.output(fmt.Sprintf("File size: %d\nMD5 hash: %x", fileSize, getHash(t.Filepath)))
	numChunks := ceil(fileSize, CHUNKSIZE)

	bytesLeft := fileSize
	var i int64

	ticker := time.NewTicker(time.Millisecond * 1000)
	go func() {
		for _ = range ticker.C {
			percentDone := 100 * float64(float64(fileSize)-float64(bytesLeft)) / float64(fileSize)
			t.updateProgressBar(int(percentDone))
		}
	}()

	// transmit filename and size
	filename := filepath.Base(t.Filepath)
	filenameLen := len(filename)
	err = binary.Write(t.Conn, binary.BigEndian, filenameLen)
	if err != nil {
		n.teardown(t)
		t.output(fmt.Sprintf("Error writing filename length: %s\n Please quit and restart Flying Carpet.", err))
		sendChan <- false
		return
	}
	_, err = t.Conn.Write([]byte(filename))
	if err != nil {
		n.teardown(t)
		t.output(fmt.Sprintf("Error writing filename: %s\n Please quit and restart Flying Carpet.", err))
		sendChan <- false
		return
	}
	err = binary.Write(t.Conn, binary.BigEndian, fileSize)	
	if err != nil {
		n.teardown(t)
		t.output(fmt.Sprintf("Error transmitting file size: %s\n Please quit and restart Flying Carpet.", err))
		sendChan <- false
		return
	}
	/////////////////////////////

	for i = 0; i < numChunks; i++ {
		bufferSize := min(CHUNKSIZE, bytesLeft)
		buffer := make([]byte, bufferSize)
		bytesRead, err := file.Read(buffer)
		if int64(bytesRead) != bufferSize {
			n.teardown(t)
			t.output(fmt.Sprintf("bytesRead: %d\nbufferSize: %d\n", bytesRead, bufferSize))
			t.output("Error reading out file. Please quit and restart Flying Carpet.")
			sendChan <- false
			return
		}
		bytesLeft -= bufferSize

		// encrypt buffer
		encryptedBuffer := encrypt(buffer, t.Passphrase)

		// send size of buffer
		chunkSize := int64(len(encryptedBuffer))
		err = binary.Write(t.Conn, binary.BigEndian, chunkSize)
		if err != nil {
			n.teardown(t)
			t.output("Error writing chunk length. Please quit and restart Flying Carpet.")
			sendChan <- false
			return
		}

		// send buffer
		bytes, err := t.Conn.Write(encryptedBuffer)
		if bytes != len(encryptedBuffer) {
			n.teardown(t)
			t.output("Send error. Please quit and restart Flying Carpet.")
			sendChan <- false
			return
		}
	}
	if runtime.GOOS == "darwin" {
		t.AdHocChan <- false
		<-t.AdHocChan
	}
	ticker.Stop()
	t.updateProgressBar(100)
	t.output(fmt.Sprintf("Sending took %s\n", time.Since(start)))
	sendChan <- true
	return
}

func (t *Transfer) receiveAndAssemble(receiveChan chan bool, n Network) {
	start := time.Now()
	defer t.Conn.Close()

	// receive filename and size
	var filenameLen int64
	err := binary.Read(t.Conn, binary.BigEndian, &filenameLen)
	if err != nil {
		n.teardown(t)
		t.output(fmt.Sprintf("Error receiving filename length: %s\nPlease quit and restart Flying Carpet.",err))
		receiveChan <- false
		return
	}
	filenameBytes := make([]byte, filenameLen)	
	_, err = io.ReadFull(t.Conn, filenameBytes)
	if err != nil {
		n.teardown(t)
		t.output(fmt.Sprintf("Error receiving filename: %s\nPlease quit and restart Flying Carpet.",err))
		receiveChan <- false
		return
	}
	filename := string(filenameBytes)
	var fileSize int64
	err = binary.Read(t.Conn, binary.BigEndian, &fileSize)
	if err != nil {
		n.teardown(t)
		t.output(fmt.Sprintf("Error receiving file size: %s\nPlease quit and restart Flying Carpet.",err))
		receiveChan <- false
		return
	}
	// TODO: set textbox to filename and add progress bar
	if _, err := os.Open(t.Filepath + filename); err != nil {
		t.Filepath += filename
	} else {
		t.Filepath = t.Filepath + t.SSID + "_" + filename
	}
	t.output(fmt.Sprintf("Filename: %s\nFile size: %i", filename, fileSize))
	t.updateFilename(t.Filepath)
	/////////////////////////////
	
	// os.Remove(t.Filepath)
	outFile, err := os.OpenFile(t.Filepath, os.O_CREATE|os.O_RDWR, 0666)
	if err != nil {
		n.teardown(t)
		t.output("Error creating out file. Please quit and restart Flying Carpet.")
		receiveChan <- false
		return
	}
	defer outFile.Close()

	for {
		// get chunk size
		var chunkSize int64
		err := binary.Read(t.Conn, binary.BigEndian, &chunkSize)
		if err != nil {
			t.output(fmt.Sprintf("err: %s", err.Error()))
		}
		if chunkSize == 0 {
			// done receiving
			t.Conn.Close()
			break
		}

		// get chunk
		chunk := make([]byte, chunkSize)
		bytesReceived, err := io.ReadFull(t.Conn, chunk)
		if err != nil {
			t.output("Error reading from stream. Retrying.")
			t.output(err.Error())
			continue
			// n.teardown(t)
			// t.output("Error reading from stream. Please quit and restart Flying Carpet. " + err.Error())
			// receiveChan <- false
			// return
		}
		if int64(bytesReceived) != chunkSize {
			t.output(fmt.Sprintf("bytesReceived: %d\nchunkSize: %d", bytesReceived, chunkSize))
		}

		// decrypt and add to outfile
		decryptedChunk := decrypt(chunk, t.Passphrase)
		_, err = outFile.Write(decryptedChunk)
		if err != nil {
			n.teardown(t)
			t.output("Error writing to out file. Please quit and restart Flying Carpet.")
			receiveChan <- false
			return
		}
	}
	if runtime.GOOS == "darwin" && t.Peer == "windows" {
		t.AdHocChan <- false
		<-t.AdHocChan
	}
	t.output(fmt.Sprintf("Received file size: %d", getSize(outFile)))
	t.output(fmt.Sprintf("Received file hash: %x", getHash(t.Filepath)))
	t.output(fmt.Sprintf("Receiving took %s", time.Since(start)))

	speed := (float64(getSize(outFile)*8) / 1000000) / (float64(time.Since(start)) / 1000000000)
	t.output(fmt.Sprintf("Speed: %.2fmbps", speed))
	// signal main that it's okay to return
	receiveChan <- true
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

func (t *Transfer) updateProgressBar(percentage int) {
	progressEvt := wx.NewThreadEvent(wx.EVT_THREAD, PROGRESS_BAR_UPDATE)
	progressEvt.SetInt(percentage)
	t.Frame.QueueEvent(progressEvt)
}

func (t *Transfer) showProgressBar() {
	progressEvt := wx.NewThreadEvent(wx.EVT_THREAD, PROGRESS_BAR_SHOW)
	t.Frame.QueueEvent(progressEvt)
}

func (t *Transfer) updateFilename(filename string) {
	filenameEvt := wx.NewThreadEvent(wx.EVT_THREAD, RECEIVE_FILE_UPDATE)
	filenameEvt.SetString(filename)
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
