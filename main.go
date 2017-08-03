package main

import (
	"bufio"
	"crypto/md5"
	"flag"
	"fmt"
	"log"
	"math/rand"
	"net"
	"os"
	"runtime"
	"strconv"
	"strings"
	"time"
)

const DIAL_TIMEOUT = 60
const JOIN_ADHOC_TIMEOUT = 60
const FIND_MAC_TIMEOUT = 60

func main() {

	if len(os.Args) == 1 {
		printUsage()
		return
	}

	var p_outFile = flag.String("send", "", "File to be sent.")
	var p_inFile = flag.String("receive", "", "Destination path of file to be received.")
	var p_port = flag.Int("port", 3290, "TCP port to use (must match on both ends).")
	var p_peer = flag.String("peer", "", "Use \"-peer mac\" or \"-peer windows\" to match the other computer.")
	flag.Parse()
	outFile := *p_outFile
	inFile := *p_inFile
	port := *p_port
	peer := *p_peer

	receiveChan := make(chan bool)
	sendChan := make(chan bool)

	if peer == "" || ( peer != "mac" && peer != "windows" ) {
		log.Fatal("Must choose [ -peer mac ] or [ -peer windows ].")
	}
	t := Transfer{
		Port:       port,
		Peer:       peer,
		AdHocChan:	make(chan bool),
	}
	var n Network

	// sending
	if outFile != "" && inFile == "" {
		t.Passphrase = getPassword()
		pwBytes := md5.Sum([]byte(t.Passphrase))
		prefix := pwBytes[:3]
		t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)
		t.Filepath = outFile

		if runtime.GOOS == "windows" {
			w := WindowsNetwork{Mode: "sending"}
			w.PreviousSSID = w.getCurrentWifi()
			n = w
		} else if runtime.GOOS == "darwin" {
			n = MacNetwork{Mode: "sending"}
		}
		n.connectToPeer(&t)

		if connected := t.sendFile(sendChan, n); connected == false {
			fmt.Println("Could not establish TCP connection with peer")
			return
		}
		<-sendChan
		fmt.Println("Send complete, resetting WiFi and exiting.")

	//receiving
	} else if inFile != "" && outFile == "" {
		t.Passphrase = generatePassword()
		pwBytes := md5.Sum([]byte(t.Passphrase))
		prefix := pwBytes[:3]
		t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)
		fmt.Printf("=============================\n" +
			"Transfer password: %s\nPlease use this password on sending end when prompted to start transfer.\n" +
			"=============================\n",t.Passphrase)

		if runtime.GOOS == "windows" {
			n = WindowsNetwork{Mode: "receiving"}
		} else if runtime.GOOS == "darwin" {
			n = MacNetwork{Mode: "receiving"}
		}
		n.connectToPeer(&t)

		t.Filepath = inFile
		go t.receiveFile(receiveChan, n)
		// wait for listener to be up
		<-receiveChan
		// wait for reception to finish
		<-receiveChan
		fmt.Println("Reception complete, resetting WiFi and exiting.")
	} else {
		printUsage()
		return
	}
	n.resetWifi(&t)
}

func (t *Transfer) receiveFile(receiveChan chan bool, n Network) {
	ln, err := net.Listen("tcp", ":"+strconv.Itoa(t.Port))
	if err != nil {
		n.teardown(t)
		log.Fatal("Could not listen on :",t.Port)
	}
	fmt.Println("Listening on", ":"+strconv.Itoa(t.Port))
	receiveChan <- true
	for {
		conn, err := ln.Accept()
		if err != nil {
			n.teardown(t)
			log.Fatal("Error accepting connection on :",t.Port)
		}
		t.Conn = conn
		fmt.Println("Connection accepted")
		go t.receiveAndAssemble(receiveChan, n)
	}
}

func (t *Transfer) sendFile(sendChan chan bool, n Network) bool {
	var conn net.Conn
	var err error
	for i := 0; i < DIAL_TIMEOUT; i++ {
		err = nil
		conn, err = net.DialTimeout("tcp", t.RecipientIP+":"+strconv.Itoa(t.Port), time.Millisecond * 10)
		if err != nil {
			fmt.Printf("\rFailed connection %2d to %s, retrying.", i, t.RecipientIP)
			time.Sleep(time.Second * 1)
			continue
		} else {
			fmt.Printf("\n")
			t.Conn = conn
			go t.chunkAndSend(sendChan, n)
			return true
		}
	}
	fmt.Printf("Waited %d seconds, no connection. Exiting.", DIAL_TIMEOUT)
	return false
}

func generatePassword() string {
	const chars = "0123456789abcdefghijkmnopqrstuvwxyzABCDEFGHIJKLMNPQRSTUVWXYZ"
	rand.Seed(time.Now().UTC().UnixNano())
	pwBytes := make([]byte, 8)
	for i := range pwBytes {
		pwBytes[i] = chars[rand.Intn(len(chars))]
	}
	return string(pwBytes)
}

func getPassword() (pw string) {
	reader := bufio.NewReader(os.Stdin)
	fmt.Print("Enter password from receiving end: ")
	pw,err := reader.ReadString('\n')
	if err != nil {
		panic("Error getting password.")
	}
	pw = strings.TrimSpace(pw)
	return
}

func printUsage() {
	fmt.Println("\nUsage (Windows): flyingcarpet.exe -send ./picture.jpg -peer mac")
	fmt.Println("[Enter password from receiving end.]\n")
	fmt.Println("Usage (Mac): ./flyingcarpet -receive ./newpicture.jpg -peer windows")
	fmt.Println("[Enter password into sending end.]\n")
	return
}

type Transfer struct {
	Filepath    string
	Passphrase  string
	SSID        string
	Conn        net.Conn
	Port        int
	RecipientIP string
	Peer        string
	AdHocChan	chan bool
}

type Network interface {
	connectToPeer(*Transfer)
	getCurrentWifi() string
	resetWifi(*Transfer)
	teardown(*Transfer)
}

type WindowsNetwork struct {
	Mode         string // sending or receiving
	PreviousSSID string
}

type MacNetwork struct {
	Mode string // sending or receiving
}
