package main

import (
	"errors"
	"io"
	"os"

	rice "github.com/GeertJohan/go.rice"
)

// used if running CLI version as the wifi direct
// dll won't have been bundled with the GUI
func writeDLL() (string, error) {
	// use rice to bundle
	box, err := rice.FindBox("static")
	if err != nil {
		return "", errors.New("error locating box: " + err.Error())
	}

	// get handle to dll from box
	file, err := box.Open("wfd.dll")
	if err != nil {
		return "", errors.New("error getting file from box: " + err.Error())
	}
	defer file.Close()

	// find suitable location to write dll and complete filepath
	tempLoc := os.TempDir()
	if tempLoc == "" {
		tempLoc, err = os.Executable()
		if err != nil {
			return "", errors.New("error finding suitable location to write dll: " + err.Error())
		}
	}
	tempLoc = tempLoc + string(os.PathSeparator) + "wfd.dll"

	// delete preexisting dll, create new one, and write it
	os.Remove(tempLoc)
	outputFile, err := os.Create(tempLoc)
	if err != nil {
		return "", errors.New("error creating dll: " + err.Error())
	}
	defer outputFile.Close()
	_, err = io.Copy(outputFile, file)
	if err != nil {
		return "", errors.New("error writing embedded data to output file: " + err.Error())
	}
	return tempLoc, err
}
