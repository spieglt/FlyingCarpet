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
func (cli *Cli) Output(message string) {
	fmt.Println(message)
}

// ShowProgressBar is a placeholder to fulfill the UI interface from core.
func (cli *Cli) ShowProgressBar() {}

// UpdateProgressBar prints the status of a file transfer.
func (cli *Cli) UpdateProgressBar(percentDone int) {
	fmt.Printf("\rProgress: %3d%%", percentDone)
	if percentDone == 100 {
		fmt.Println()
	}
}

// ToggleStartButton is a placeholder to fulfill the UI interface from core.
func (cli *Cli) ToggleStartButton() {}

// ShowPwPrompt is a placeholder to fulfill the UI interface from core.
func (cli *Cli) ShowPwPrompt() bool { return false }

func getInput(cli *Cli) *core.Transfer {
	if core.HostOS == "windows" {
		adminCheck(cli)
	}

	// get flags
	if len(os.Args) == 1 {
		printUsage()
		os.Exit(1)
	}
	var pSend = flag.Bool("send", false, "Use this flag to send files. Globs accepted. Put filenames with spaces in quotes.")
	var pReceive = flag.Bool("receive", false, "Use this flag to receive files, and provide the path of a destination folder.")
	var pPort = flag.Int("port", 3290, "TCP port to use (must match on both ends).")
	var pPeer = flag.String("peer", "", "Use \"-peer linux\", \"-peer mac\", or \"-peer windows\" to match the other computer.")
	flag.Parse()
	send := *pSend
	receive := *pReceive
	port := *pPort
	peer := *pPeer

	// main transfer object that will be handed to core
	t := &core.Transfer{}

	// validate mode flags
	if send == receive {
		printUsage()
		log.Fatal("Must choose mode, [ -send | -receive ].")
	}

	// validate peer flag
	switch peer {
	case "linux":
		t.Peer = peer
	case "mac":
		t.Peer = peer
	case "windows":
		t.Peer = peer
	default:
		printUsage()
		log.Fatal("Must choose a [ -peer linux|mac|windows ].")
	}

	// fill out transfer struct
	t.Port = port
	t.WfdSendChan, t.WfdRecvChan = make(chan string), make(chan string)
	t.Ctx, t.CancelCtx = context.WithCancel(context.Background())

	// parse flags
	var err error
	if send {
		t.Mode = "sending"
		baseList := flag.Args()
		for _, filename := range baseList {
			expandedList, err := filepath.Glob(filename)
			if err != nil {
				printUsage()
				log.Fatalf("Error expanding glob %s: %s\n", filename, err)
			}
			for _, v := range expandedList {
				v, err = filepath.Abs(v)
				if err != nil {
					printUsage()
					log.Fatalf("Error getting abs path for %s: %s", v, err)
				}
				t.FileList = append(t.FileList, v)
			}
		}
		if len(t.FileList) == 0 {
			printUsage()
			log.Fatalf("No files found to send! Please enter filename(s) after arguments (globs/wildcards accepted).")
		}
	} else if receive {
		t.Mode = "receiving"
		if flag.Arg(0) == "" {
			printUsage()
			log.Fatalf("Receive flag was chosen but no destination folder was specified.")
		}
		path, err := filepath.Abs(flag.Arg(0))
		if err != nil {
			cli.Output(fmt.Sprintf("Error getting abs path for %s: %s", flag.Arg(0), err))
			os.Exit(1)
		}
		fpStat, err := os.Stat(flag.Arg(0))
		if err != nil || !fpStat.IsDir() {
			cli.Output("Please select valid folder.")
			os.Exit(1)
		}
		t.ReceiveDir = path + string(os.PathSeparator)
	}

	// make sure DLL is available
	location, err := writeDLL()
	if err != nil {
		cli.Output("Error writing WiFi Direct dll to temp location: " + err.Error())
		os.Exit(1)
	}
	t.DllLocation = location

	// deal with password
	if t.Mode == "sending" {
		t.Password = getPassword()
	} else if t.Mode == "receiving" {
		t.Password = core.GeneratePassword()
	}

	return t
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
	fmt.Println("\nTo send files:")
	fmt.Println("(Windows) $ .\\flyingcarpet.exe -send -peer mac pic1.jpg pic35.jpg \"filename with spaces.docx\" *.txt")
	fmt.Println("[Enter password from receiving end.]")
	fmt.Println("\nTo receive files:")
	fmt.Println("  (Mac)   $ ./flyingcarpet -receive -peer windows ~/Downloads")
	fmt.Println("[Enter password into sending end.]\n")
	return
}
