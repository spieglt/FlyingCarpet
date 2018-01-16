package main

import (
	"crypto/md5"
	"fmt"
	"github.com/dontpanic92/wxGo/wx"
	"math/rand"
	"net"
	"runtime"
	"strconv"
	"time"
)

const DIAL_TIMEOUT = 60
const JOIN_ADHOC_TIMEOUT = 60
const FIND_MAC_TIMEOUT = 60

type Transfer struct {
	Filepath       string
	Passphrase     string
	SSID           string
	RecipientIP    string
	Peer           string // "mac", "windows", or "linux"
	Mode           string // "sending" or "receiving"
	PreviousSSID   string
	Port           int
	AdHocCapable   bool
	AdHocChan      chan bool
	WifiDirectChan chan string
	Conn           net.Conn
	Frame          *MainFrame
	// cli?
}

func main() {
	wx1 := wx.NewApp("Flying Carpet")
	mf := newGui()
	mf.Show()
	wx1.MainLoop()
	return
}

func mainRoutine(t *Transfer) {
	receiveChan := make(chan bool)
	sendChan := make(chan bool)
	wfdc := make(chan string)
	t.WifiDirectChan = wfdc

	defer func() {
		enableStartButton(t)
		resetWifi(t)
	}()

	if t.Mode == "send" {
		pwBytes := md5.Sum([]byte(t.Passphrase))
		prefix := pwBytes[:3]
		t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)

		if runtime.GOOS == "windows" {
			t.PreviousSSID = getCurrentWifi(t)
		} else if runtime.GOOS == "linux" {
			t.PreviousSSID = getCurrentUUID(t)
		}
		if !connectToPeer(t) {
			t.output("Aborting transfer.")
			return
		}

		if connected := sendFile(sendChan, t); !connected {
			t.output("Could not establish TCP connection with peer. Aborting transfer.")
			return
		}
		t.output("Connected")
		sendSuccess := <-sendChan
		if !sendSuccess {
			t.output("Aborting transfer.")
			return
		}
		t.output("Send complete, resetting WiFi and exiting.")

	} else if t.Mode == "receive" {
		t.Passphrase = generatePassword()
		pwBytes := md5.Sum([]byte(t.Passphrase))
		prefix := pwBytes[:3]
		t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)

		showPassphraseEvt := wx.NewThreadEvent(wx.EVT_THREAD, POP_UP_PASSWORD)
		showPassphraseEvt.SetString(t.Passphrase)
		t.Frame.QueueEvent(showPassphraseEvt)
		t.output(fmt.Sprintf("=============================\n"+
			"Transfer password: %s\nPlease use this password on sending end when prompted to start transfer.\n"+
			"=============================\n", t.Passphrase))

		if connectToPeer(t) {
			t.output("Aborting transfer.")
			return
		}

		go receiveFile(receiveChan, t)
		// wait for listener to be up
		listenerIsUp := <-receiveChan
		if !listenerIsUp {
			t.output("Aborting transfer.")
			return
		}
		// wait for reception to finish
		receiveSuccess := <-receiveChan
		if !receiveSuccess {
			t.output("Aborting transfer.")
			return
		}
		t.output("Reception complete, resetting WiFi and exiting.")
	}
}

func receiveFile(receiveChan chan bool, t *Transfer) {

	ln, err := net.Listen("tcp", ":"+strconv.Itoa(t.Port))
	if err != nil {
		resetWifi(t)
		t.output(fmt.Sprintf("Could not listen on :%d. Err: %s", t.Port, err))
		receiveChan <- false
		return
	}
	t.output("Listening on :" + strconv.Itoa(t.Port))
	receiveChan <- true

	conn, err := ln.Accept()
	if err != nil {
		resetWifi(t)
		t.output(fmt.Sprintf("Error accepting connection on :%d", t.Port))
		receiveChan <- false
		return
	}
	t.Conn = conn
	t.output("Connection accepted")
	go receiveAndAssemble(receiveChan, t, &ln)
}

func sendFile(sendChan chan bool, t *Transfer) bool {

	var conn net.Conn
	var err error
	t.output("Trying to connect to " + t.RecipientIP + " for " + strconv.Itoa(DIAL_TIMEOUT) + " seconds.")
	for i := 0; i < DIAL_TIMEOUT; i++ {
		err = nil
		conn, err = net.DialTimeout("tcp", t.RecipientIP+":"+strconv.Itoa(t.Port), time.Millisecond*10)
		if err != nil {
			// t.output(fmt.Sprintf("Failed connection %2d to %s, retrying.", i, t.RecipientIP))
			time.Sleep(time.Second * 1)
			continue
		}
		t.output("Successfully dialed peer.")
		t.Conn = conn
		go chunkAndSend(sendChan, t)
		return true
	}
	t.output(fmt.Sprintf("Waited %d seconds, no connection.", DIAL_TIMEOUT))
	return false
}

func generatePassword() string {
	// no l, I, or O because they look too similar to each other, 1, and 0
	const chars = "0123456789abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ"
	rand.Seed(time.Now().UTC().UnixNano())
	pwBytes := make([]byte, 4)
	for i := range pwBytes {
		pwBytes[i] = chars[rand.Intn(len(chars))]
	}
	return string(pwBytes)
}
