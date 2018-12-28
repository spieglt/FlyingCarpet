package core

import (
	"context"
	"crypto/md5"
	"errors"
	"fmt"
	"math/rand"
	"net"
	"runtime"
	"strconv"
	"time"
)

const hostOS = runtime.GOOS

// Transfer holds all information necessary to send or receive files
type Transfer struct {
	FileList     []string
	ReceiveDir   string
	Password     string
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
}

// UI interface provides methods to accept information
// from and update the user
type UI interface {
	Output(string)
	ShowProgressBar()
	UpdateProgressBar(int)
	ToggleStartButton()
}

// StartTransfer is the main routine, invoked by cli and gui.
func StartTransfer(t *Transfer, ui UI) {
	var err error

	// cleanup
	defer func() {
		ui.ToggleStartButton()
		resetWifi(t, ui)
	}()

	// get ssid
	pwBytes := md5.Sum([]byte(t.Password))
	prefix := pwBytes[:3]
	t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)

	if t.Mode == "sending" {
		// to stop searching for ad hoc network (if Mac jumps off)
		if hostOS == "darwin" {
			defer func() { t.CancelCtx() }()
		}

		// not necessary for mac as it reaches for its most preferred network automatically
		if hostOS == "windows" {
			t.PreviousSSID = getCurrentWifi(ui)
		} else if hostOS == "linux" {
			t.PreviousSSID = getCurrentUUID()
		}

		// make ip connection
		if err = connectToPeer(t, ui); err != nil {
			ui.Output(err.Error())
			ui.Output("Aborting transfer.")
			return
		}

		// make tcp connection
		conn, err := dialPeer(t, ui)
		if conn != nil {
			defer conn.Close()
		}
		if err != nil {
			ui.Output(err.Error())
			ui.Output("Could not establish TCP connection with peer. Aborting transfer.")
			return
		}
		ui.Output("Connected")

		// tell receiving end how many files we're sending
		if err = sendCount(conn, len(t.FileList)); err != nil {
			ui.Output("Could not send number of files: " + err.Error())
			return
		}

		// send files
		for i, v := range t.FileList {
			if len(t.FileList) > 1 {
				ui.Output("=============================")
				ui.Output(fmt.Sprintf("Beginning transfer %d of %d. Filename: %s", i+1, len(t.FileList), v))
			}
			if err = send(conn, t, i, ui); err != nil {
				ui.Output(err.Error())
				ui.Output("Aborting transfer.")
				return
			}
		}
		ui.Output("Send complete, resetting WiFi and exiting.")

	} else if t.Mode == "receiving" {
		// why the && here? because if we're on darwin and receiving from darwin, we'll be hosting the adhoc and thus haven't joined it,
		// and thus don't need to shut down the goroutine trying to stay on it. does this need to happen when peer is linux? yes.
		if hostOS == "darwin" && (t.Peer == "windows" || t.Peer == "linux") {
			defer func() {
				t.CancelCtx()
			}()
		}

		ui.Output(fmt.Sprintf("=============================\n"+
			"Transfer password: %s\nPlease use this password on sending end when prompted to start transfer.\n"+
			"=============================\n", t.Password))

		// make ip connection
		if err = connectToPeer(t, ui); err != nil {
			ui.Output(err.Error())
			ui.Output("Aborting transfer.")
			return
		}

		// make tcp connection
		listener, conn, err := listenForPeer(t, ui)
		// wait till end to close listener and tcp connection for multi-file transfers
		// need to defer one func that closes both iff each != nil
		defer func() {
			if conn != nil {
				if err := conn.Close(); err != nil {
					ui.Output("Error closing TCP connection: " + err.Error())
				}

			}
			if listener != nil {
				if err := (*listener).Close(); err != nil {
					ui.Output("Error closing TCP listener: " + err.Error())
				}
			}
		}()

		if err != nil {
			ui.Output(err.Error())
			ui.Output("Aborting transfer.")
			return
		}

		// find out how many files we're receiving
		numFiles, err := receiveCount(conn)
		if err != nil {
			ui.Output("Could not receive number of files: " + err.Error())
			return
		}

		// receive files
		for i := 0; i < numFiles; i++ {
			if numFiles > 1 {
				ui.Output("=============================")
				ui.Output(fmt.Sprintf("Receiving file %d of %d.", i+1, numFiles))
			}
			if err = receive(conn, t, i, ui); err != nil {
				ui.Output(err.Error())
				ui.Output("Aborting transfer.")
				return
			}
		}

		ui.Output("Reception complete, resetting WiFi and exiting.")
	}
}

func listenForPeer(t *Transfer, ui UI) (*net.TCPListener, net.Conn, error) {
	ln, err := net.ListenTCP("tcp", &net.TCPAddr{Port: t.Port})
	if err != nil {
		return nil, nil, fmt.Errorf("Could not listen on :%d. Err: %s", t.Port, err)
	}
	ui.Output("Listening on :" + strconv.Itoa(t.Port))
	// accept times out every second so that this function can receive context cancellation
	for {
		select {
		case <-t.Ctx.Done():
			return nil, nil, errors.New("Exiting listenForPeer, transfer was canceled.")
		default:
			ln.SetDeadline(time.Now().Add(time.Second))
			conn, err := ln.Accept()
			if err != nil {
				ui.Output("Error accepting connection: " + err.Error())
				continue
			}
			ui.Output("Connection accepted")
			return ln, conn, nil
		}
	}
}

func dialPeer(t *Transfer, ui UI) (conn net.Conn, err error) {
	ui.Output("Trying to connect to " + t.RecipientIP)
	for i := 0; i < 1000; i++ {
		select {
		case <-t.Ctx.Done():
			return nil, errors.New("Exiting dialPeer, transfer was canceled.")
		default:
			err = nil
			conn, err = net.DialTimeout("tcp", t.RecipientIP+":"+strconv.Itoa(t.Port), time.Millisecond*50)
			if err != nil {
				ui.Output(fmt.Sprintf("Failed connection %2d to %s, retrying.", i, t.RecipientIP))
				time.Sleep(time.Second * 1)
				continue
			}
			ui.Output("Successfully dialed peer.")
			return conn, nil
		}
	}
	return nil, fmt.Errorf("Could not dial peer.")
}

// GeneratePassword returns a 4 char password to display on the receiving end and enter into the sending end
func GeneratePassword() string {
	// no l, I, or O because they look too similar to each other, 1, and 0
	const chars = "0123456789abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ"
	rand.Seed(time.Now().UTC().UnixNano())
	pwBytes := make([]byte, 4)
	for i := range pwBytes {
		pwBytes[i] = chars[rand.Intn(len(chars))]
	}
	return string(pwBytes)
}
