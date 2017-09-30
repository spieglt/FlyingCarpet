package main

import (
	"crypto/md5"
	"encoding/binary"
	"fmt"
	"github.com/dontpanic92/wxGo/wx"
	"io"
	"os"
	"runtime"
	"time"
)

const CHUNKSIZE = 1000000 // 1MB

func (t *Transfer) chunkAndSend(sendChan chan bool, n Network) {

	outputEvent := wx.NewThreadEvent(wx.EVT_THREAD, OUTPUT_BOX_UPDATE)
	progressEvent := wx.NewThreadEvent(wx.EVT_THREAD, PROGRESS_BAR_UPDATE)
	progressShow := wx.NewThreadEvent(wx.EVT_THREAD, PROGRESS_BAR_SHOW)
	t.Frame.QueueEvent(progressShow)

	start := time.Now()
	defer t.Conn.Close()

	file, err := os.Open(t.Filepath)
	if err != nil {
		n.teardown(t)
		outputEvent.SetString("\nError opening out file. Please quit and restart Flying Carpet.")
		t.Frame.QueueEvent(outputEvent)
		sendChan <- false
		return
	}
	defer file.Close()

	fileSize := getSize(file)
	outputEvent.SetString(fmt.Sprintf("File size: %d\nMD5 hash: %x", fileSize, getHash(t.Filepath)))
	t.Frame.QueueEvent(outputEvent)

	numChunks := ceil(fileSize, CHUNKSIZE)

	bytesLeft := fileSize
	var i int64
	outputEvent.SetString("\n")
	t.Frame.QueueEvent(outputEvent)
	for i = 0; i < numChunks; i++ {
		bufferSize := min(CHUNKSIZE, bytesLeft)
		buffer := make([]byte, bufferSize)
		bytesRead, err := file.Read(buffer)
		if int64(bytesRead) != bufferSize {
			n.teardown(t)
			outputEvent.SetString(fmt.Sprintf("bytesRead: %d\nbufferSize: %d\n", bytesRead, bufferSize))
			t.Frame.QueueEvent(outputEvent)
			outputEvent.SetString("\nError reading out file. Please quit and restart Flying Carpet.")
			t.Frame.QueueEvent(outputEvent)
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
			outputEvent.SetString("\nError writing chunk length. Please quit and restart Flying Carpet.")
			t.Frame.QueueEvent(outputEvent)
			sendChan <- false
			return
		}

		// send buffer
		bytes, err := t.Conn.Write(encryptedBuffer)
		if bytes != len(encryptedBuffer) {
			n.teardown(t)
			outputEvent.SetString("\nSend error. Please quit and restart Flying Carpet.")
			t.Frame.QueueEvent(outputEvent)
			sendChan <- false
			return
		}
		percentDone := (fileSize - bytesLeft) / fileSize
		progressEvent.SetInt(int(percentDone))
		t.Frame.QueueEvent(progressEvent)
	}
	if runtime.GOOS == "darwin" {
		t.AdHocChan <- false
		<-t.AdHocChan
	}
	outputEvent.SetString(fmt.Sprintf("\nSending took %s\n", time.Since(start)))
	t.Frame.QueueEvent(outputEvent)
	sendChan <- true
	return
}

func (t *Transfer) receiveAndAssemble(receiveChan chan bool, n Network) {
	outputEvent := wx.NewThreadEvent(wx.EVT_THREAD, OUTPUT_BOX_UPDATE)
	start := time.Now()
	defer t.Conn.Close()
	os.Remove(t.Filepath)

	outFile, err := os.OpenFile(t.Filepath, os.O_CREATE|os.O_RDWR, 0666)
	if err != nil {
		n.teardown(t)
		outputEvent.SetString("\nError creating out file. Please quit and restart Flying Carpet.")
		t.Frame.QueueEvent(outputEvent)
		receiveChan <- false
		return
	}
	defer outFile.Close()

	for {
		// get chunk size
		var chunkSize int64
		err := binary.Read(t.Conn, binary.BigEndian, &chunkSize)
		if err != nil {
			outputEvent.SetString(fmt.Sprintf("\nerr: %s", err.Error()))
			t.Frame.QueueEvent(outputEvent)
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
			n.teardown(t)
			outputEvent.SetString("\nError reading from stream. Please quit and restart Flying Carpet.")
			t.Frame.QueueEvent(outputEvent)
			receiveChan <- false
			return
		}
		if int64(bytesReceived) != chunkSize {
			outputEvent.SetString(fmt.Sprintf("\nbytesReceived: %d\nchunkSize: %d", bytesReceived, chunkSize))
			t.Frame.QueueEvent(outputEvent)
		}

		// decrypt and add to outfile
		decryptedChunk := decrypt(chunk, t.Passphrase)
		_, err = outFile.Write(decryptedChunk)
		if err != nil {
			n.teardown(t)
			outputEvent.SetString("\nError writing to out file. Please quit and restart Flying Carpet.")
			t.Frame.QueueEvent(outputEvent)
			receiveChan <- false
			return
		}
	}
	outputEvent.SetString(fmt.Sprintf("\nReceived file size: %d", getSize(outFile)))
	t.Frame.QueueEvent(outputEvent)
	outputEvent.SetString(fmt.Sprintf("\nReceived file hash: %x", getHash(t.Filepath)))
	t.Frame.QueueEvent(outputEvent)
	outputEvent.SetString(fmt.Sprintf("\nReceiving took %s", time.Since(start)))
	t.Frame.QueueEvent(outputEvent)
	speed := (float64(getSize(outFile)*8) / 1000000) / (float64(time.Since(start)) / 1000000000)
	outputEvent.SetString(fmt.Sprintf("\nSpeed: %.2fmbps", speed))
	t.Frame.QueueEvent(outputEvent)
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
