package core

//#include <stdlib.h>
import "C"
import (
	"fmt"
	"syscall"
	"time"
	"unsafe"
)

var dll *syscall.DLL

// TODO: error handling

func startLegacyAP(t *Transfer, ui UI) {
	if dll == nil {
		var err error
		dll, err = syscall.LoadDLL(".\\wfd.dll")
		if err != nil {
			t.WfdRecvChan <- fmt.Sprintf("Loading DLL failed: %s", err)
			return
		}
	}

	ConsoleInit, err := dll.FindProc("GoConsoleInit")
	if err != nil {
		ui.Output(err.Error())
	}
	ConsoleFree, err := dll.FindProc("GoConsoleFree")
	if err != nil {
		ui.Output(err.Error())
	}
	ExecuteCommand, err := dll.FindProc("GoConsoleExecuteCommand")
	if err != nil {
		ui.Output(err.Error())
	}

	cInitRes, _, initErr := ConsoleInit.Call()
	initRes := int(cInitRes)
	if initRes == 0 {
		t.WfdRecvChan <- fmt.Sprintf("Initializing Windows Runtime for Wi-Fi Direct failed: %s", initErr)
		return
	} else if initRes == 1 {
		ui.Output("Initialized Windows Runtime.")
	} else {
		t.WfdRecvChan <- fmt.Sprintf("Something went wrong with initializing Windows Runtime: %d.", initRes)
		return
	}

	ssid := unsafe.Pointer(C.CString("ssid " + t.SSID))
	password := unsafe.Pointer(C.CString("pass " + t.Password + t.Password))
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

	t.WfdRecvChan <- "started"
	// in loop, listen on chan to commands from rest of program
	for {
		select {
		case msg, ok := <-t.WfdSendChan:
			if !ok || msg == "quit" {
				cFreeRes, _, _ := ConsoleFree.Call()
				freeRes := int(cFreeRes)
				if freeRes == 0 {
					ui.Output("Failed to uninitialize Windows Runtime.")
				}
			}
			t.WfdRecvChan <- "Wifi-Direct stopped."
			return
		// TODO: cause of process leak? not releasing dll till 3 seconds after program is closed?
		default:
			time.Sleep(time.Second * 3)
		}
	}
	// err = dll.Release()
	// if err != nil {
	// 	ui.Output(fmt.Sprintf("Error releasing DLL: %s", err))
	// }
}
