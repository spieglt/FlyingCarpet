package main

/*
#cgo CFLAGS: -x objective-c -fobjc-arc
#cgo LDFLAGS: -framework Cocoa
#include "AppDelegate.h"
#include "AppDelegate.m"
#include "ViewController.h"
#include "ViewController.m"
#include "main.m"
#include "receiver.m"
*/
import "C"

import (
    "fmt"
    //"syscall"
    //"unsafe"
)

func main() {

    cs := C.hey()
    fmt.Println(C.GoString(cs))
    C.main(0,nil)
    return
}
