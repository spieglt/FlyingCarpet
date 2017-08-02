package main

import (
	"crypto/md5"
	"encoding/binary"
	"fmt"
	"io"
	"log"
	"math"
	"os"
	"time"
)

const CHUNKSIZE = 1000000

func (t *Transfer) chunkAndSend(sendChan chan bool, n Network) {
	start := time.Now()
	defer t.Conn.Close()

	file, err := os.Open(t.Filepath)
	if err != nil {
		n.teardown(t)
		log.Fatal("Error opening out file.")
	}
	defer file.Close()

	fileSize := getSize(file)
	fmt.Printf("File size: %d\n", fileSize)
	fmt.Printf("MD5 hash: %x\n", getHash(t.Filepath))

	// problem with using this this way? https://mrekucci.blogspot.com/2015/07/dont-abuse-mathmax-mathmin.html
	numChunks := uint64(math.Ceil(float64(fileSize) / float64(CHUNKSIZE)))

	bytesLeft := fileSize
	var i uint64
	for i = 0; i < numChunks; i++ {
		bufferSize := int64(math.Min(float64(CHUNKSIZE), float64(bytesLeft)))
		buffer := make([]byte, bufferSize)
		bytesRead, err := file.Read(buffer)
		if int64(bytesRead) != bufferSize {
			n.teardown(t)
			fmt.Printf("bytesRead: %d\nbufferSize: %d\n", bytesRead, bufferSize)
			log.Fatal("Error reading out file.")
		}
		bytesLeft -= bufferSize

		// encrypt buffer
		encryptedBuffer := encrypt(buffer, t.Passphrase)

		// send size of buffer
		chunkSize := int64(len(encryptedBuffer))
		err = binary.Write(t.Conn, binary.BigEndian, chunkSize)
		if err != nil {
			n.teardown(t)
			log.Fatal("Error writing chunk length.")
		}

		// send buffer
		bytes, err := t.Conn.Write(encryptedBuffer)
		if bytes != len(encryptedBuffer) {
			n.teardown(t)
			log.Fatal("Send error.")
		}

		fmt.Printf("\rProgress: %3.0f%%", (float64(fileSize)-float64(bytesLeft))/float64(fileSize)*100)
	}
	t.AdHocChan <- false
	fmt.Printf("\nSending took %s\n", time.Since(start))
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
		log.Fatal("Error creating out file.")
	}
	defer outFile.Close()

	for {
		// get chunk size
		var chunkSize int64
		err := binary.Read(t.Conn, binary.BigEndian, &chunkSize)
		if err != nil {
			fmt.Println("err:", err)
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
			log.Fatal("Error reading from stream.")
		}
		if int64(bytesReceived) != chunkSize {
			fmt.Println("bytesReceived:", bytesReceived)
			fmt.Println("chunkSize:", chunkSize)
		}

		// decrypt and add to outfile
		decryptedChunk := decrypt(chunk, t.Passphrase)
		_, err = outFile.Write(decryptedChunk)
		if err != nil {
			n.teardown(t)
			log.Fatal("Error writing to out file.")
		}
	}
	fmt.Println("Received file size: ", getSize(outFile))
	fmt.Printf("Received file hash: %x\n", getHash(t.Filepath))
	fmt.Printf("Receiving took %s\n", time.Since(start))
	speed := (float64(getSize(outFile)*8) / 1000000) / float64(time.Since(start)/1000000000)
	fmt.Printf("Speed: %.2fmbps\n", speed)
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
