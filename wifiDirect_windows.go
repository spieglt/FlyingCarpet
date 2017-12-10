package main

//#include <stdlib.h>
import "C"
import (
	"fmt"
	"os"
	"os/exec"
	"strings"
	"syscall"
	"time"
	"unsafe"
)

// TODO: error handling, use chan
func (n *Network) startLegacyAP(t *Transfer, startChan chan bool) {
	// echo %TEMP%
	cmd := exec.Command("cmd", "/C", "echo %TEMP%")
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	cmdBytes, err := cmd.CombinedOutput()
	if err != nil {
		t.output("Error getting temp location.")
		startChan <- false
		return
	}
	tmpLoc := strings.TrimSpace(string(cmdBytes)) + "\\wfd.dll"
	t.output(tmpLoc)

	// write dll to file
	os.Remove(tmpLoc)

	data, err := Asset("static/wfd.dll")
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

	// Use DLL
	dll := syscall.NewLazyDLL(tmpLoc)

	ConsoleInit := dll.NewProc("GoConsoleInit")
	ConsoleFree := dll.NewProc("GoConsoleFree")
	ExecuteCommand := dll.NewProc("GoConsoleExecuteCommand")

	ConsoleInit.Call()

	ssid := unsafe.Pointer(C.CString("ssid " + t.SSID))
	password := unsafe.Pointer(C.CString("pass " + t.Passphrase))
	autoaccept := unsafe.Pointer(C.CString("autoaccept 1"))
	start := unsafe.Pointer(C.CString("start"))
	stop := unsafe.Pointer(C.CString("stop"))

	defer C.free(ssid)
	defer C.free(password)
	defer C.free(autoaccept)
	defer C.free(start)
	defer C.free(stop)

	ExecuteCommand.Call(uintptr(ssid))
	ExecuteCommand.Call(uintptr(password))
	ExecuteCommand.Call(uintptr(autoaccept))
	ExecuteCommand.Call(uintptr(start))

	startChan <- true
	// in loop, listen on chan to commands from rest of program
	for {
		select {
		case msg, ok := <-n.WifiDirectChan:
			if !ok || msg == "quit" {
				ConsoleFree.Call()
			}
			n.WifiDirectChan <- "Wifi-Direct stopped."
			return
		default:
			time.Sleep(time.Second * 3)
		}
	}

	return
}

func bail(err error, startChan chan bool, t *Transfer, n *Network) {
	t.output(fmt.Sprintf("Bailing: %s", err))
	startChan <- false
	n.teardown(t)
}
