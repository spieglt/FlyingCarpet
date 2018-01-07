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

var dll *syscall.DLL

func (n *Network) startLegacyAP(t *Transfer, startChan chan bool) {
	cmd := exec.Command("cmd", "/C", "echo %USERPROFILE%")
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	cmdBytes, err := cmd.CombinedOutput()
	if err != nil {
		t.output("Error getting temp location.")
		startChan <- false
		return
	}
	tmpLoc := strings.TrimSpace(string(cmdBytes)) + "\\AppData\\Local\\Temp\\wfd.dll"

	if dll == nil {
		// write dll to file
		err = os.Remove(tmpLoc)
		if err != nil {
			t.output(err.Error())
		}
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
		dll, err = syscall.LoadDLL(tmpLoc)
		if err != nil {
			t.output(fmt.Sprintf("Loading DLL failed: %s", err))
			startChan <- false
			n.resetWifi(t)
			return
		}
	}

	ConsoleInit, err := dll.FindProc("GoConsoleInit")
	if err != nil {
		t.output(err.Error())
	}
	ConsoleFree, err := dll.FindProc("GoConsoleFree")
	if err != nil {
		t.output(err.Error())
	}
	ExecuteCommand, err := dll.FindProc("GoConsoleExecuteCommand")
	if err != nil {
		t.output(err.Error())
	}

	cInitRes, _, initErr := ConsoleInit.Call()
	initRes := int(cInitRes)
	if initRes == 0 {
		t.output(fmt.Sprintf("Initializing Windows Runtime for Wi-Fi Direct failed: %s", initErr))
		startChan <- false
		n.resetWifi(t)
		return
	} else if initRes == 1 {
		t.output("Initialized Windows Runtime.")
	} else {
		t.output(fmt.Sprintf("Something went wrong with initializing Windows Runtime: %d.", initRes))
		startChan <- false
		n.resetWifi(t)
		return
	}

	ssid := unsafe.Pointer(C.CString("ssid " + t.SSID))
	password := unsafe.Pointer(C.CString("pass " + t.Passphrase + t.Passphrase))
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
				cFreeRes, _, _ := ConsoleFree.Call()
				freeRes := int(cFreeRes)
				if freeRes == 0 {
					t.output("Failed to uninitialize Windows Runtime.")
				}
			}
			n.WifiDirectChan <- "Wifi-Direct stopped."
			return
		default:
			time.Sleep(time.Second * 3)
		}
	}
	// err = dll.Release()
	// if err != nil {
	// 	t.output(fmt.Sprintf("Error releasing DLL: %s", err))
	// }
	return
}

func bail(err error, startChan chan bool, t *Transfer, n *Network) {
	t.output(fmt.Sprintf("Bailing: %s", err))
	startChan <- false
	n.resetWifi(t)
}
