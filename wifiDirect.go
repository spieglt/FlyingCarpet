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

// TODO: error handling
func (w *WindowsNetwork) startLegacyAP(t *Transfer) {
	// write legacyAP bin to file
	tmpLoc := ".\\wdlap.exe"
	os.Remove(tmpLoc)

	data, err := Asset("static/wdlap.exe")
	if err != nil {
		t.output(fmt.Sprintf("Static file error: %s", err))
		w.teardown(t)
		// return false
	}
	outFile, err := os.OpenFile(tmpLoc, os.O_CREATE|os.O_RDWR, 0744)
	if err != nil {
		t.output("Error creating temp file")
		w.teardown(t)
		// return false
	}
	if _, err = outFile.Write(data); err != nil {
		t.output("Write error")
		w.teardown(t)
		// return false
	}
	outFile.Close()
	defer os.Remove(tmpLoc)

	// run it with proper options
	cmd := exec.Command(tmpLoc)
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	stdin, err := cmd.StdinPipe()
	if err != nil {
		t.output(err.Error())
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		t.output(err.Error())
	}
	reader := bufio.NewReader(stdout)
	if err != nil {
		t.output(err.Error())
	}
	err = cmd.Start()
	if err != nil {
		t.output(err.Error())
	}

	go readStdout(reader, t)
	io.WriteString(stdin, "ssid "+t.SSID+"\n")
	io.WriteString(stdin, "pass "+t.Passphrase+"\n")
	io.WriteString(stdin, "autoaccept 1\n")
	io.WriteString(stdin, "start\n")

	// in loop, listen on chan to commands from rest of program
	for {
		select {
		case msg, ok := <-w.wifiDirectChan:
			if !ok || msg == "quit" {
				io.WriteString(stdin, "quit\n")
			}
			w.wifiDirectChan <- "Exiting WifiDirect."
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
