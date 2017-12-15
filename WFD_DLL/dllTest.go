package main

//#include <stdlib.h>
import "C"
import (
	"fmt"
	"os"
	"syscall"
	"unsafe"
	"bufio"
)

func main() {
	dll := syscall.NewLazyDLL("C:\\Users\\Theron\\source\\repos\\WFD_DLL\\x64\\Debug\\WFD_DLL.dll")

	ConsoleInit := dll.NewProc("GoConsoleInit")
	ConsoleFree := dll.NewProc("GoConsoleFree")
	ExecuteCommand := dll.NewProc("GoConsoleExecuteCommand")

	a, b, err := ConsoleInit.Call()
	fmt.Printf("a: %# x\nb: %# x\nerr: %s\n", a, b, err)

	ssid		:= unsafe.Pointer(C.CString("ssid tester"))
	password	:= unsafe.Pointer(C.CString("pass testing123"))
	autoaccept	:= unsafe.Pointer(C.CString("autoaccept 1"))
	start		:= unsafe.Pointer(C.CString("start"))
	stop		:= unsafe.Pointer(C.CString("stop"))

	defer C.free(ssid)
	defer C.free(password)
	defer C.free(autoaccept)
	defer C.free(start)
	defer C.free(stop)

	ExecuteCommand.Call(uintptr(start))

	reader := bufio.NewReader(os.Stdin)

	fmt.Printf("Press enter to stop WFD.\n")
	reader.ReadString('\n')
	ConsoleFree.Call()

	fmt.Printf("Press enter to reinit.\n")
	reader.ReadString('\n')
	ConsoleInit.Call()

	fmt.Printf("Press enter to restart.\n")
	reader.ReadString('\n')
	ExecuteCommand.Call(uintptr(start))

	fmt.Printf("Press enter to quit.\n")
	reader.ReadString('\n')
	ConsoleFree.Call()
	return
}