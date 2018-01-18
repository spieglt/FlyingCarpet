package main

import (
	"context"
	"crypto/md5"
	"errors"
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
	Ctx            context.Context
	CancelCtx      context.CancelFunc
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
	wfdc := make(chan string)
	t.WifiDirectChan = wfdc
	var err error

	defer func() {
		enableStartButton(t)
		resetWifi(t)
	}()

	if t.Mode == "sending" {

		defer func() {
			if runtime.GOOS == "darwin" {
				t.CancelCtx()
			}
		}()

		pwBytes := md5.Sum([]byte(t.Passphrase))
		prefix := pwBytes[:3]
		t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)

		if runtime.GOOS == "windows" {
			t.PreviousSSID = getCurrentWifi(t)
		} else if runtime.GOOS == "linux" {
			t.PreviousSSID = getCurrentUUID(t)
		}
		if err = connectToPeer(t); err != nil {
			t.output("Aborting transfer.")
			return
		}

		if err = dialPeer(t); err != nil {
			t.output(err.Error())
			t.output("Could not establish TCP connection with peer. Aborting transfer.")
			return
		}
		t.output("Connected")

		if err = chunkAndSend(t); err != nil {
			t.output(err.Error())
			t.output("Aborting transfer.")
			return
		}
		t.output("Send complete, resetting WiFi and exiting.")

	} else if t.Mode == "receiving" {
		defer func() {
			// why the && here? because if we're on darwin and receiving from darwin, we'll be hosting the adhoc and thus haven't joined it,
			// and thus don't need to shut down the goroutine trying to stay on it. does this need to happen when peer is linux? yes.
			if runtime.GOOS == "darwin" && (t.Peer == "windows" || t.Peer == "linux") {
				t.CancelCtx()
			}
		}()
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

		if err = connectToPeer(t); err != nil {
			t.output("Aborting transfer.")
			return
		}
		listener, err := listenForPeer(t)
		// wait till end to close listener for multi-file transfers
		if listener != nil {
			defer (*listener).Close()
		}
		if err != nil {
			t.output(err.Error())
			t.output("Aborting transfer.")
			return
		}

		if err = receiveAndAssemble(t); err != nil {
			t.output(err.Error())
			t.output("Aborting transfer.")
			return
		}

		t.output("Reception complete, resetting WiFi and exiting.")
	}
}

func listenForPeer(t *Transfer) (*net.Listener, error) {
	ln, err := net.Listen("tcp", ":"+strconv.Itoa(t.Port))
	if err != nil {
		return nil, errors.New(fmt.Sprintf("Could not listen on :%d. Err: %s", t.Port, err))
	}
	t.output("Listening on :" + strconv.Itoa(t.Port))
	conn, err := ln.Accept()
	if err != nil {
		return nil, errors.New(fmt.Sprintf("Error accepting connection on :%d", t.Port))
	}
	t.Conn = conn
	t.output("Connection accepted")
	return &ln, nil
}

func dialPeer(t *Transfer) error {
	var conn net.Conn
	var err error
	t.output("Trying to connect to " + t.RecipientIP + " for " + strconv.Itoa(DIAL_TIMEOUT) + " seconds.")
	for i := 0; i < DIAL_TIMEOUT; i++ {
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting dialPeer, transfer was canceled.")
		default:
			err = nil
			conn, err = net.DialTimeout("tcp", t.RecipientIP+":"+strconv.Itoa(t.Port), time.Millisecond*10)
			if err != nil {
				// t.output(fmt.Sprintf("Failed connection %2d to %s, retrying.", i, t.RecipientIP))
				time.Sleep(time.Second * 1)
				continue
			}
			t.output("Successfully dialed peer.")
			t.Conn = conn
			return nil
		}
	}
	return errors.New(fmt.Sprintf("Waited %d seconds, no connection.", DIAL_TIMEOUT))
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
