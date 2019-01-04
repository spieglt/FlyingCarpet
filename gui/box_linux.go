package main

import (
	"os/exec"

	rice "github.com/GeertJohan/go.rice"
)

func getBox() (*rice.Box, error) {
	return rice.FindBox("./flyingcarpet/deploy/linux")
}

func runCommand(programName string) {
	exec.Command("chmod", "+x", programName).Run()
	exec.Command(programName).Start()
}
