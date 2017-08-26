package main

/*
#cgo CFLAGS: -x objective-c -fobjc-arc
#cgo LDFLAGS: -framework Cocoa
#include "AppDelegate.h"
#include "AppDelegate.m"
#include "ViewController.h"
#include "ViewController.m"
//#include "main.m"
#include "receiver.m"
*/
import "C"

import (
    "fmt"
    "io/ioutil"
    "os"
    //"syscall"
    //"unsafe"
)

func main() {
	// go other()
 	// cs := C.hey()
 	// fmt.Println(C.GoString(cs))

	file, err := os.OpenFile("/tmp/fc.fifo", os.O_RDONLY, os.ModeNamedPipe)
	if err != nil {
		panic(err)
	}
	msg,err := ioutil.ReadAll(file)
	if err != nil {
		panic(err)
	}
	fmt.Println(string(msg))
	file.Close()
    return
}

func other() {
	cs := C.ho()
	fmt.Println(C.GoString(cs))
	return
}