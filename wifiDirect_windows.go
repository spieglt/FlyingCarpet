package main

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strings"
	"syscall"
	"time"
)

// TODO: error handling, use chan
func (n *Network) startLegacyAP(t *Transfer, startChan chan bool) {
	// write legacyAP bin to file
	tmpLoc := ".\\wdlap.exe"
	os.Remove(tmpLoc)

	data, err := Asset("static/wdlap.exe")
	if err != nil {
		bail(err, startChan, t, n)
		return
	}
	outFile, err := os.OpenFile(tmpLoc, os.O_CREATE|os.O_RDWR, 0744)
	if err != nil {
		bail(err, startChan, t, n)
		return
	}
	if _, err = outFile.Write(data); err != nil {
		bail(err, startChan, t, n)
		return
	}
	outFile.Close()
	defer os.Remove(tmpLoc)

	// run it with proper options
	cmd := exec.Command(tmpLoc)
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	defer cmd.Process.Kill()
	stdin, err := cmd.StdinPipe()
	if err != nil {
		bail(err, startChan, t, n)
		return
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		bail(err, startChan, t, n)
		return
	}
	reader := bufio.NewReader(stdout)
	if err != nil {
		bail(err, startChan, t, n)
		return
	}
	err = cmd.Start()
	if err != nil {
		bail(err, startChan, t, n)
		return
	}

	go readStdout(reader, t)
	io.WriteString(stdin, "ssid "+t.SSID+"\n")
	io.WriteString(stdin, "pass "+t.Passphrase+"\n")
	io.WriteString(stdin, "autoaccept 1\n")
	io.WriteString(stdin, "start\n")

	startChan <- true
	// in loop, listen on chan to commands from rest of program
	for {
		select {
		case msg, ok := <-n.wifiDirectChan:
			if !ok || msg == "quit" {
				io.WriteString(stdin, "quit\n")
			}
			n.wifiDirectChan <- "Exiting WifiDirect."
			return
		default:
			time.Sleep(time.Second * 3)
		}
	}

}

func readStdout(reader *bufio.Reader, t *Transfer) {
	for {
		resp, err := reader.ReadString('\n')
		if err != nil {
			t.output(fmt.Sprintf("WifiDirect stdout error: %s", err))
			// return
			time.Sleep(time.Second * 3)
		}
		if resp != "\r\n" && resp != ">\r\n" {
			// write to window
			t.output(strings.TrimSpace(string(resp)))
		}
	}
}

func bail(err error, startChan chan bool, t *Transfer, n *Network) {
	t.output(fmt.Sprintf("Bailing: %s", err))
	startChan <- false
	n.teardown(t)
}
