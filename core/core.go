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

	"golang.org/x/crypto/scrypt"
)

const HostOS = runtime.GOOS

// Transfer holds all information necessary to send or receive files
type Transfer struct {
	FileList       []string
	ReceiveDir     string
	Password       string
	HashedPassword []byte
	SSID           string
	RecipientIP    string
	Peer           string // "mac", "windows", or "linux"
	Mode           string // "sending" or "receiving"
	PreviousSSID   string
	DllLocation    string
	Port           int
	AdHocCapable   bool
	Ctx            context.Context
	CancelCtx      context.CancelFunc
	WfdSendChan    chan string
	WfdRecvChan    chan string
}

// UI interface provides methods to accept information
// from and update the user
type UI interface {
	Output(string)
	ShowProgressBar()
	UpdateProgressBar(int)
	ToggleStartButton()
	ShowPwPrompt() bool
}

// StartTransfer is the main routine, invoked by cli and gui.
func StartTransfer(t *Transfer, ui UI) {
	var err error

	// cleanup
	defer func() {
		ui.UpdateProgressBar(0)
		ui.ToggleStartButton()
		resetWifi(t, ui)
	}()

	// get ssid
	pwBytes := md5.Sum([]byte(t.Password))
	prefix := pwBytes[:3]
	t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)

	salt := pwBytes[3:11]
	t.HashedPassword, err = scrypt.Key([]byte(t.Password), salt, 1<<15, 8, 1, 32)
	if err != nil {
		ui.Output("Error hashing password")
		return
	}

	ui.Output("\nStarting Transfer\n=============================")
	if t.Mode == "sending" {
		// to stop searching for ad hoc network (if Mac jumps off)
		if HostOS == "darwin" {
			defer func() { t.CancelCtx() }()
		}

		// not necessary for mac as it reaches for its most preferred network automatically
		if HostOS == "windows" {
			t.PreviousSSID = getCurrentWifi(ui)
		} else if HostOS == "linux" {
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
			if err = sendFile(conn, t, i, ui); err != nil {
				ui.Output(err.Error())
				ui.Output("Aborting transfer.")
				return
			}
		}
		ui.Output("=============================\n")
		ui.Output("Send complete, resetting WiFi and exiting.")

	} else if t.Mode == "receiving" {
		// why the && here? because if we're on darwin and receiving from darwin, we'll be hosting the adhoc and thus haven't joined it,
		// and thus don't need to shut down the goroutine trying to stay on it. does this need to happen when peer is linux? yes.
		if HostOS == "darwin" && (t.Peer == "windows" || t.Peer == "linux") {
			defer func() {
				t.CancelCtx()
			}()
		}

		ui.Output(fmt.Sprintf("Transfer password: %s\nPlease use this password on sending end when prompted to start transfer.\n"+
			"=============================\n", t.Password))

		// make ip connection
		if err = connectToPeer(t, ui); err != nil {
			ui.Output(err.Error())
			ui.Output("Aborting transfer.")
			return
		}

		// make tcp connection
		listener, conn, err := listenForPeer(t, ui)
		defer func() {
			if conn != nil {
				if err := conn.Close(); err != nil {
					ui.Output("Error closing TCP connection: " + err.Error())
				} else {
					ui.Output("Closed TCP connection")
				}
			}
			if listener != nil {
				if err := (*listener).Close(); err != nil {
					ui.Output("Error closing TCP listener: " + err.Error())
				} else {
					ui.Output("Closed TCP listener")
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
			if err = receiveFile(conn, t, i, ui); err != nil {
				ui.Output(err.Error())
				ui.Output("Aborting transfer.")
				return
			}
		}

		ui.Output("=============================\n")
		ui.Output("Reception complete, resetting WiFi and exiting.")
	}
}

func listenForPeer(t *Transfer, ui UI) (ln *net.TCPListener, conn net.Conn, err error) {
	ln, err = net.ListenTCP("tcp", &net.TCPAddr{Port: t.Port})
	if err != nil {
		return nil, nil, fmt.Errorf("Could not listen on :%d. Err: %s", t.Port, err)
	}
	ui.Output("Listening on :" + strconv.Itoa(t.Port) + ", waiting for connection....")
	// accept times out every second so that this function can receive context cancellation
	for {
		select {
		case <-t.Ctx.Done():
			return ln, conn, errors.New("Exiting listenForPeer, transfer was canceled")
		default:
			ln.SetDeadline(time.Now().Add(time.Second))
			conn, err = ln.Accept()
			if err != nil {
				// ui.Output("Error accepting connection: " + err.Error())
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
			return nil, errors.New("Exiting dialPeer, transfer was canceled")
		default:
			err = nil
			conn, err = net.DialTimeout("tcp", t.RecipientIP+":"+strconv.Itoa(t.Port), time.Millisecond*500)
			if err != nil {
				ui.Output(fmt.Sprintf("Failed connection %2d to %s, retrying.", i, t.RecipientIP))
				time.Sleep(time.Second * 1)
				continue
			}
			ui.Output("Successfully dialed peer.")
			return conn, nil
		}
	}
	return nil, fmt.Errorf("Could not dial peer")
}

// GeneratePassword returns a 4 char password to display on the receiving end and enter into the sending end
func GeneratePassword() string {
	// no l, I, 0, or O, because they look too similar
	const chars = "23456789abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ"
	rand.Seed(time.Now().UTC().UnixNano())
	pwBytes := make([]byte, 4)
	for i := range pwBytes {
		pwBytes[i] = chars[rand.Intn(len(chars))]
	}
	return string(pwBytes)
}

const AboutMessage = `https://github.com/spieglt/flyingcarpet
Version: 2.1
Copyright (c) 2020, Theron Spiegl. All rights reserved.
Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.
* Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.
* Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

Flying Carpet performs encrypted file transfers between two computers with wireless cards via ad hoc WiFi (or Wi-Fi Direct if necessary). No access point, router, or other networking gear is required. Just select a file, whether each computer is sending or receiving, and the operating system of the other computer. Flying Carpet will do its best to restore your wireless settings afterwards, but if there is an error, you may have to rejoin your wireless network manually. Thanks for using it and please provide feedback on GitHub!`
