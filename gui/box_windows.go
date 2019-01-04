package main

import (
	"os/exec"
	"syscall"

	rice "github.com/GeertJohan/go.rice"
)

func getBox() (*rice.Box, error) {
	return rice.FindBox(".\\flyingcarpet\\deploy\\windows")
}

func runCommand(programName string) {
	// "cmd /C" is necessary because it will error with "process requires elevation" if launched from non-admin shell
	// but "cmd /C" will make a console window appear for the life of the program, so we hide it with syscall.
	cmd := exec.Command("cmd", "/C", programName+".exe")
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	cmd.Start() // Start() == don't wait for it to return
}
