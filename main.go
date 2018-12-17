package main

import (
	"context"
	"crypto/md5"
	"errors"
	"fmt"
	"math/rand"
	"net"
	"os"
	"runtime"
	"strconv"
	"time"

	"github.com/therecipe/qt/widgets"
)

const dialTimeout = 60
const joinAdHocTimeout = 60
const findMacTimeout = 60
const hostOS = runtime.GOOS

type mainFrame struct{}

// The Transfer struct holds transfer-specific data used throughout program.
// Should reorganize/clean this up but not sure how best to do so.
type Transfer struct {
	Filepath     string
	FileList     []string
	Passphrase   string
	SSID         string
	RecipientIP  string
	Peer         string // "mac", "windows", or "linux"
	Mode         string // "sending" or "receiving"
	PreviousSSID string
	Port         int
	AdHocCapable bool
	Ctx          context.Context
	CancelCtx    context.CancelFunc
	WfdSendChan  chan string
	WfdRecvChan  chan string
	Frame        *mainFrame
}

func main() {
	app := widgets.NewQApplication(len(os.Args), os.Args)
	showWindow()
	app.Exec()
	return
}

func mainRoutine(t *Transfer) {
	t.WfdSendChan, t.WfdRecvChan = make(chan string), make(chan string)
	var err error

	// cleanup
	defer func() {
		// enableStartButton(t)
		resetWifi(t)
	}()

	if t.Mode == "sending" {
		// to stop searching for ad hoc network (if Mac jumps off)
		defer func() {
			if hostOS == "darwin" {
				t.CancelCtx()
			}
		}()

		pwBytes := md5.Sum([]byte(t.Passphrase))
		prefix := pwBytes[:3]
		t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)

		if hostOS == "windows" {
			t.PreviousSSID = getCurrentWifi(t)
		} else if hostOS == "linux" {
			t.PreviousSSID = getCurrentUUID(t)
		}

		// make ip connection
		if err = connectToPeer(t); err != nil {
			t.output(err.Error())
			t.output("Aborting transfer.")
			return
		}

		// make tcp connection
		conn, err := dialPeer(t)
		if conn != nil {
			defer (*conn).Close()
		}
		if err != nil {
			t.output(err.Error())
			t.output("Could not establish TCP connection with peer. Aborting transfer.")
			return
		}
		t.output("Connected")

		// tell receiving end how many files we're sending
		if err = sendCount(conn, t); err != nil {
			t.output("Could not send number of files: " + err.Error())
			return
		}

		// send files
		for i, v := range t.FileList {
			if len(t.FileList) > 1 {
				t.output("=============================")
				t.output(fmt.Sprintf("Beginning transfer %d of %d. Filename: %s", i+1, len(t.FileList), v))
			}
			t.Filepath = v
			if err = chunkAndSend(conn, t); err != nil {
				t.output(err.Error())
				t.output("Aborting transfer.")
				return
			}
		}

		t.output("Send complete, resetting WiFi and exiting.")

	} else if t.Mode == "receiving" {
		defer func() {
			// why the && here? because if we're on darwin and receiving from darwin, we'll be hosting the adhoc and thus haven't joined it,
			// and thus don't need to shut down the goroutine trying to stay on it. does this need to happen when peer is linux? yes.
			if hostOS == "darwin" && (t.Peer == "windows" || t.Peer == "linux") {
				t.CancelCtx()
			}
		}()

		t.Passphrase = generatePassword()
		pwBytes := md5.Sum([]byte(t.Passphrase))
		prefix := pwBytes[:3]
		t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)

		// showPassphraseEvt
		// t.Frame.QueueEvent(showPassphraseEvt)
		t.output(fmt.Sprintf("=============================\n"+
			"Transfer password: %s\nPlease use this password on sending end when prompted to start transfer.\n"+
			"=============================\n", t.Passphrase))

		// make ip connection
		if err = connectToPeer(t); err != nil {
			t.output(err.Error())
			t.output("Aborting transfer.")
			return
		}

		// make tcp connection
		listener, conn, err := listenForPeer(t)
		// wait till end to close listener and tcp connection for multi-file transfers
		// need to defer one func that closes both iff each != nil
		defer func() {
			if conn != nil {
				if err := (*conn).Close(); err != nil {
					t.output("Error closing TCP connection: " + err.Error())
				}

			}
			if listener != nil {
				if err := (*listener).Close(); err != nil {
					t.output("Error closing TCP listener: " + err.Error())
				}
			}
		}()

		if err != nil {
			t.output(err.Error())
			t.output("Aborting transfer.")
			return
		}

		// find out how many files we're receiving
		numFiles, err := receiveCount(conn, t)
		if err != nil {
			t.output("Could not receive number of files: " + err.Error())
			return
		}

		// receive files
		for i := 0; i < numFiles; i++ {
			if numFiles > 1 {
				t.output("=============================")
				t.output(fmt.Sprintf("Receiving file %d of %d.", i+1, numFiles))
			}
			if err = receiveAndAssemble(conn, t); err != nil {
				t.output(err.Error())
				t.output("Aborting transfer.")
				return
			}
		}

		t.output("Reception complete, resetting WiFi and exiting.")
	}
}

func listenForPeer(t *Transfer) (*net.TCPListener, *net.Conn, error) {
	ln, err := net.ListenTCP("tcp", &net.TCPAddr{Port: t.Port})
	if err != nil {
		return nil, nil, fmt.Errorf("Could not listen on :%d. Err: %s", t.Port, err)
	}
	t.output("Listening on :" + strconv.Itoa(t.Port))

	for {
		select {
		case <-t.Ctx.Done():
			return nil, nil, errors.New("Exiting listenForPeer, transfer was canceled.")
		default:
			ln.SetDeadline(time.Now().Add(time.Second))
			conn, err := ln.Accept()
			if err != nil {
				// t.output("Error accepting connection: " + err.Error())
				continue
			}
			t.output("Connection accepted")
			return ln, &conn, nil
		}
	}
}

func dialPeer(t *Transfer) (*net.Conn, error) {
	var conn net.Conn
	var err error
	t.output("Trying to connect to " + t.RecipientIP + " for " + strconv.Itoa(dialTimeout) + " seconds.")
	for i := 0; i < dialTimeout; i++ {
		select {
		case <-t.Ctx.Done():
			return nil, errors.New("Exiting dialPeer, transfer was canceled.")
		default:
			err = nil
			conn, err = net.DialTimeout("tcp", t.RecipientIP+":"+strconv.Itoa(t.Port), time.Millisecond*10)
			if err != nil {
				// t.output(fmt.Sprintf("Failed connection %2d to %s, retrying.", i, t.RecipientIP))
				time.Sleep(time.Second * 1)
				continue
			}
			t.output("Successfully dialed peer.")
			return &conn, nil
		}
	}
	return nil, fmt.Errorf("Waited %d seconds, no connection.", dialTimeout)
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
