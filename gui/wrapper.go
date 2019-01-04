package main

import (
	"errors"
	"fmt"
	"io"
	"log"
	"os"
	"path/filepath"

	rice "github.com/GeertJohan/go.rice"
)

// newCopyToTempWalkFunc returns a filepath.WalkFunc for use with go.rice.
// It copies the tree of files embedded in the rice box to the destination.
func newCopyToTempWalkFunc(tempLoc string, box *rice.Box) filepath.WalkFunc {
	return func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return errors.New("error from previous iteration of walk function: " + err.Error())
		}
		// fmt.Println(path)
		// open the embedded file
		file, err := box.Open(path)
		defer file.Close()
		if err != nil {
			return errors.New("error opening embedded file: " + err.Error())
		}
		// if the embedded file is a directory, create it
		if info.IsDir() {
			os.Mkdir(tempLoc+string(os.PathSeparator)+path, 0755)
		} else { // else open the output file and make the main program executable
			outputFile, err := os.Create(tempLoc + string(os.PathSeparator) + path)
			defer outputFile.Close()
			if err != nil {
				return errors.New("error creating output file: " + err.Error())
			}
			if outputFile.Name() == "flyingcarpet" || outputFile.Name() == "flyingcarpet.exe" {
				outputFile.Chmod(0755)
			}
			// write the data
			_, err = io.Copy(outputFile, file) // TODO: do something with bytes written here?
			if err != nil {
				return errors.New("error writing embedded data to output file: " + err.Error())
			}
		}
		return err
	}
}

// findTempLoc tries to return temp directory, or failing that,
// the current directory, and makes a folder for the resources
func findTempLoc() string {
	subDir := "flyingcarpet"
	tempLoc := os.TempDir()
	os.RemoveAll(tempLoc + string(os.PathSeparator) + subDir)
	err := os.Mkdir(tempLoc+string(os.PathSeparator)+subDir, 0755)
	if err != nil {
		fmt.Println("Could not create directory in temp folder: " + err.Error())
		tempLoc, err = os.Executable()
		if err != nil {
			log.Fatal("Could not find a suitable place to extract Flying Carpet.")
		}
		os.RemoveAll(tempLoc + string(os.PathSeparator) + subDir)
		err = os.Mkdir(tempLoc+string(os.PathSeparator)+subDir, 0755)
		if err != nil {
			log.Fatal("Could not create directory in current folder: " + err.Error())
		}
	}
	return tempLoc + string(os.PathSeparator) + subDir
}

func main() {
	tempLoc := findTempLoc()
	fmt.Println("temp location: ", tempLoc)

	// backslashes are important for windows in go.rice,
	// and rice would panic trying to find a linux box if I branched based on
	// host OS in this file, so I had to implement getBox() in platform-specific
	// files that rice will ignore
	box, err := getBox()
	if err != nil {
		log.Fatal("error locating box: ", err)
	}

	// walks the tree of embedded files in box and writes them to the temp location
	walkFunc := newCopyToTempWalkFunc(tempLoc, box)
	err = box.Walk("", walkFunc)
	if err != nil {
		log.Fatal("error referencing box: " + err.Error())
	}

	// run flyingcarpet
	programName := tempLoc + string(os.PathSeparator) + "flyingcarpet"
	runCommand(programName)
	return
}
