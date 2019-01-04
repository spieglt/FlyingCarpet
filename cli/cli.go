package main

import (
	"bufio"
	"context"
	"flag"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"strings"

	"github.com/spieglt/flyingcarpet/core"
)

// Cli fulfills the UI interface to be used in the core functions
type Cli struct{}

// Output prints messages. It's a function to fulfill the UI interface from core.
func (cli Cli) Output(message string) {
	fmt.Println(message)
}

// ShowProgressBar is a placeholder to fulfill the UI interface from core.
func (cli Cli) ShowProgressBar() {}

// UpdateProgressBar prints the status of a file transfer.
func (cli Cli) UpdateProgressBar(percentDone int) {
	fmt.Printf("\rProgress: %3d%%", percentDone)
}

// ToggleStartButton is a placeholder to fulfill the UI interface from core.
func (cli Cli) ToggleStartButton() {}

// ShowPwPrompt is a placeholder to fulfill the UI interface from core.
func (cli Cli) ShowPwPrompt() bool { return false }

func getInput(cli *Cli) *core.Transfer {
	if core.HostOS == "windows" {
		adminCheck(cli)
	}

	// get flags
	if len(os.Args) == 1 {
		printUsage()
		os.Exit(1)
	}
	var pSendFiles = flag.String("send", "", "File to be sent. (Use [ -send multi ] for multiple files, and list files/globs after other flags.)\n\n"+
		"Example (Windows): .\\flyingcarpet.exe -send multi -peer mac pic1.jpg pic35.jpg \"filename with spaces.docx\" *.txt\n"+
		"Example (macOS/Linux): ./flyingcarpet -send multi -peer windows movie.mp4 ../*.mp3\n")
	var pReceiveDir = flag.String("receive", "", "Destination directory for files to be received.")
	var pPort = flag.Int("port", 3290, "TCP port to use (must match on both ends).")
	var pPeer = flag.String("peer", "", "Use \"-peer linux\", \"-peer mac\", or \"-peer windows\" to match the other computer.")
	flag.Parse()
	sendFiles := *pSendFiles
	receiveDir := *pReceiveDir
	port := *pPort
	peer := *pPeer

	// main transfer object that will be handed to core
	t := &core.Transfer{}

	// validate peer flag
	switch peer {
	case "linux":
		t.Peer = peer
	case "mac":
		t.Peer = peer
	case "windows":
		t.Peer = peer
	default:
		log.Fatal("Must choose [ -peer linux|mac|windows ].")
	}

	// fill out transfer struct
	t.Port = port
	t.WfdSendChan, t.WfdRecvChan = make(chan string), make(chan string)
	t.Ctx, t.CancelCtx = context.WithCancel(context.Background())

	// parse flags
	var err error
	if sendFiles == "" && receiveDir != "" { // receiving
		t.Mode = "receiving"
		path, err := filepath.Abs(receiveDir)
		if err != nil {
			cli.Output(fmt.Sprintf("Error getting abs path for %s: %s", receiveDir, err))
		}
		t.ReceiveDir = path + string(os.PathSeparator)
	} else if receiveDir == "" && sendFiles != "" { // sending
		t.Mode = "sending"
		t.FileList, err = parseSendFiles(sendFiles)
		if err != nil {
			cli.Output(err.Error())
			os.Exit(1)
		}
	} else {
		printUsage()
		os.Exit(1)
	}

	// deal with password
	if t.Mode == "sending" {
		t.Password = getPassword()
	} else if t.Mode == "receiving" {
		t.Password = core.GeneratePassword()
	}

	return t
}

func parseSendFiles(flagVal string) (sendFiles []string, err error) {
	if flagVal == "multi" { // -send multi
		baseList := flag.Args()
		// var finalList []string
		for _, filename := range baseList {
			expandedList, err := filepath.Glob(filename)
			if err != nil {
				return sendFiles, fmt.Errorf("Error expanding glob %s: %s", filename, err)
			}
			for _, v := range expandedList {
				v, err = filepath.Abs(v)
				if err != nil {
					return sendFiles, fmt.Errorf("Error getting abs path for %s: %s", v, err)
				}
				sendFiles = append(sendFiles, v)
			}
		}
		if len(sendFiles) == 0 {
			printUsage()
			return sendFiles, fmt.Errorf("No files found to send! When using [ -send multi ], list files to send after other flags. Wildcards accepted.")
		}
	} else {
		sendFiles = append(sendFiles, flagVal)
	}
	return
}

func getPassword() (pw string) {
	reader := bufio.NewReader(os.Stdin)
	fmt.Print("Enter password from receiving end: ")
	pw, err := reader.ReadString('\n')
	if err != nil {
		fmt.Println("Error getting password:", err)
	}
	pw = strings.TrimSpace(pw)
	return
}

func printUsage() {
	fmt.Println("\nSingle file usage:")
	fmt.Println("(Windows) $ flyingcarpet.exe -send ./movie.mp4 -peer mac")
	fmt.Println("[Enter password from receiving end.]")
	fmt.Println("  (Mac)   $ ./flyingcarpet -receive ./destinationFolder -peer windows")
	fmt.Println("[Enter password into sending end.]\n")

	fmt.Println("Multiple file usage:")
	fmt.Println(" (Linux)  $ ./flyingcarpet -send multi -peer windows ../Pictures/*.jpg \"Filename with spaces.txt\" movie.mp4")
	fmt.Println("[Enter password from receiving end.]")
	fmt.Println("(Windows) $ flyingcarpet.exe -receive .\\picturesFolder -peer linux")
	fmt.Println("[Enter password into sending end.]\n")
	return
}

func adminCheck(cli *Cli) {
	// inGroup := core.IsUserInAdminGroup()
	isAdmin := core.IsRunAsAdmin()
	// fmt.Printf("User in admin group: %t\n", inGroup == 1)
	// fmt.Printf("Process run as admin: %t\n", isAdmin == 1)
	if isAdmin == 0 {
		fmt.Println("Flying Carpet needs admin privileges to create/delete a firewall rule, listen on a TCP port, and clear your ARP cache. Please right-click cmd or PowerShell and select \"Run as Administrator\".")
		os.Exit(5)
	}
}
