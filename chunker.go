package main

import (
	"crypto/md5"
	"encoding/binary"
	"fmt"
	"io"
	// "log"
	"os"
	"runtime"
	"strings"
	"time"
)

const CHUNKSIZE = 1000000	// 1MB

func (t *Transfer) chunkAndSend(sendChan chan bool, n Network) {
	start := time.Now()
	defer t.Conn.Close()

	file, err := os.Open(t.Filepath)
	if err != nil {
		n.teardown(t)
		OutputBox.AppendText("\nError opening out file. Please quit and restart Flying Carpet.")
		sendChan <- false
		return
	}
	defer file.Close()

	fileSize := getSize(file)
	OutputBox.AppendText(fmt.Sprintf("File size: %d\n", fileSize))
	OutputBox.AppendText(fmt.Sprintf("MD5 hash: %x\n", getHash(t.Filepath)))

	numChunks := ceil(fileSize, CHUNKSIZE)

	bytesLeft := fileSize
	var i int64
	OutputBox.AppendText("\n")
	for i = 0; i < numChunks; i++ {
		bufferSize := min(CHUNKSIZE, bytesLeft)
		buffer := make([]byte, bufferSize)
		bytesRead, err := file.Read(buffer)
		if int64(bytesRead) != bufferSize {
			n.teardown(t)
			OutputBox.AppendText(fmt.Sprintf("bytesRead: %d\nbufferSize: %d\n", bytesRead, bufferSize))
			OutputBox.AppendText("\nError reading out file. Please quit and restart Flying Carpet.")
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
			OutputBox.AppendText("\nError writing chunk length. Please quit and restart Flying Carpet.")
			sendChan <- false
			return
		}

		// send buffer
		bytes, err := t.Conn.Write(encryptedBuffer)
		if bytes != len(encryptedBuffer) {
			n.teardown(t)
			OutputBox.AppendText("\nSend error. Please quit and restart Flying Carpet.")
			sendChan <- false
			return
		}
		OutputBox.Replace(strings.LastIndex(OutputBox.GetValue(), "\n") + 1, OutputBox.GetLastPosition(), 
			fmt.Sprintf("\nProgress: %3.0f%%", (float64(fileSize)-float64(bytesLeft))/float64(fileSize)*100))
		// fmt.Printf("\rProgress: %3.0f%%", (float64(fileSize)-float64(bytesLeft))/float64(fileSize)*100)
	}
	OutputBox.AppendText("\n")
	if runtime.GOOS == "darwin" {
		t.AdHocChan <- false
		<- t.AdHocChan
	}
	OutputBox.AppendText(fmt.Sprintf("\nSending took %s\n", time.Since(start)))
	sendChan <- true
	return
}

func (t *Transfer) receiveAndAssemble(receiveChan chan bool, n Network) {
	start := time.Now()
	defer t.Conn.Close()
	os.Remove(t.Filepath)

	outFile, err := os.OpenFile(t.Filepath, os.O_CREATE|os.O_RDWR, 0666)
	if err != nil {
		n.teardown(t)
		OutputBox.AppendText("\nError creating out file. Please quit and restart Flying Carpet.")
		receiveChan <- false
		return
	}
	defer outFile.Close()

	for {
		// get chunk size
		var chunkSize int64
		err := binary.Read(t.Conn, binary.BigEndian, &chunkSize)
		if err != nil {
			OutputBox.AppendText(fmt.Sprintf("\nerr:", err))
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
			OutputBox.AppendText("\nError reading from stream. Please quit and restart Flying Carpet.")
			receiveChan <- false
			return
		}
		if int64(bytesReceived) != chunkSize {
			OutputBox.AppendText(fmt.Sprintf("\nbytesReceived: %d", bytesReceived))
			OutputBox.AppendText(fmt.Sprintf("\nchunkSize: %d", chunkSize))
		}

		// decrypt and add to outfile
		decryptedChunk := decrypt(chunk, t.Passphrase)
		_, err = outFile.Write(decryptedChunk)
		if err != nil {
			n.teardown(t)
			OutputBox.AppendText("\nError writing to out file. Please quit and restart Flying Carpet.")
			receiveChan <- false
			return
		}
	}
	OutputBox.AppendText(fmt.Sprintf("\nReceived file size: %d", getSize(outFile)))
	OutputBox.AppendText(fmt.Sprintf("Received file hash: %x\n", getHash(t.Filepath)))
	OutputBox.AppendText(fmt.Sprintf("Receiving took %s\n", time.Since(start)))
	speed := (float64(getSize(outFile)*8) / 1000000) / (float64(time.Since(start))/1000000000)
	OutputBox.AppendText(fmt.Sprintf("Speed: %.2fmbps\n", speed))
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


func ceil(x,y int64) int64 {
	if x % y != 0 {
		return ((x/y) + 1)
	}
	return x/y
}

func min(x,y int64) int64 {
	if x < y { return x }
	return y
}