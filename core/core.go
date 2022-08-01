package core

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"errors"
	"fmt"
	"io/fs"
	"math/big"
	"net"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"time"

	"golang.org/x/crypto/scrypt"
)

const HostOS = runtime.GOOS

// Transfer holds all information necessary to send or receive files
type Transfer struct {
	FileList     []string
	ReceiveDir   string
	Password     string
	Key          []byte
	SSID         string
	RecipientIP  string
	Peer         string // "mac", "windows", "linux", or "ios"
	Mode         string // "sending" or "receiving"
	Listening    bool   // if true, this end is hosting the ad hoc network, the tcp server, and generating the password
	PreviousSSID string
	DllLocation  string
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
	pwBytes := sha256.Sum256([]byte(t.Password))
	prefix := pwBytes[:2]
	t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)

	salt := pwBytes[3:11]
	t.Key, err = scrypt.Key([]byte(t.Password), salt, 1<<15, 8, 1, 32)
	if err != nil {
		ui.Output("Error hashing password")
		return
	}

	ui.Output("\nStarting Transfer\n=============================")

	// not necessary for mac as it reaches for its most preferred network automatically
	if HostOS == "windows" {
		t.PreviousSSID = getCurrentWifi(ui)
	} else if HostOS == "linux" {
		t.PreviousSSID = getCurrentUUID()
	}

	// make ip connection
	err = connectToPeer(t, ui)
	if err != nil {
		ui.Output(err.Error())
		ui.Output("Aborting transfer.")
		return
	}

	var conn net.Conn
	// make tcp connection
	if t.Listening {
		listener, connection, err := listenForPeer(t, ui)
		conn = connection
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
	} else {
		conn, err = dialPeer(t, ui)
		if conn != nil {
			defer conn.Close()
		}
		if err != nil {
			ui.Output(err.Error())
			ui.Output("Could not establish TCP connection with peer. Aborting transfer.")
			return
		}
		ui.Output("Connected")
	}

	if t.Mode == "sending" {

		// determine if all files/folders are in the same directory
		for i := range t.FileList {
			// remove any trailing slashes
			t.FileList[i] = filepath.Clean(t.FileList[i])
		}
		usePrefix := sameDir(t.FileList)
		prefix := filepath.Dir(t.FileList[0])

		// expand any folders into list of files
		expandedList, err := getFiles(t.FileList)
		if err != nil {
			ui.Output(fmt.Sprintf("Could not access file: %s", err.Error()))
		}

		// tell receiving end how many files we're sending
		if err = sendCount(conn, len(expandedList)); err != nil {
			ui.Output("Could not send number of files: " + err.Error())
			return
		}

		// send files
		if err != nil {
			ui.Output(fmt.Sprintf("Error building file list: %s", err.Error()))
		}
		for i, v := range expandedList {
			if len(expandedList) > 1 {
				ui.Output("=============================")
				ui.Output(fmt.Sprintf("Beginning transfer %d of %d. Filename: %s", i+1, len(expandedList), v))
			}
			var relPath string
			if usePrefix && t.Peer != "ios" && t.Peer != "android" {
				relPath, err = filepath.Rel(prefix, expandedList[i])
				if err != nil {
					ui.Output(fmt.Sprintf("Error getting relative filepath: %s", err.Error()))
				}
				relPath = filepath.ToSlash(relPath)
			} else {
				relPath = filepath.Base(expandedList[i])
			}
			if err = sendFile(conn, t, expandedList, i, relPath, ui); err != nil {
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
			if err = receiveFile(conn, t, ui); err != nil {
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

// GeneratePassword returns a 6 char password to display on the receiving end and enter into the sending end
func GeneratePassword() (string, error) {
	// no l, I, 0, or O, because they look too similar
	const chars = "23456789abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ"
	upperBound := big.NewInt(int64(len(chars)))

	pwBytes := make([]byte, 6)
	for i := range pwBytes {
		idx, err := rand.Int(rand.Reader, upperBound)
		if err != nil {
			return "", err
		}
		pwBytes[i] = chars[idx.Int64()]
	}
	return string(pwBytes), nil
}

// walks folders, returning list of only files
func getFiles(paths []string) ([]string, error) {
	allFiles := make([]string, 0)
	for _, v := range paths {
		info, err := os.Stat(v)
		if err != nil {
			return nil, err
		}
		if info.IsDir() {
			filepath.WalkDir(v, func(path string, d fs.DirEntry, err error) error {
				if err != nil {
					return err
				}
				if !d.IsDir() {
					allFiles = append(allFiles, path)
				}
				return nil
			})
		} else {
			allFiles = append(allFiles, v)
		}
	}
	return allFiles, nil
}

func sameDir(paths []string) (sameDir bool) {
	// if everything is in the same Dir(), include directory info and replicate on other side
	// if this returns true, use prefix. if not, don't.
	sameDir = true
	firstPath := filepath.Dir(paths[0])
	for _, v := range paths[1:] {
		if filepath.Dir(v) != firstPath {
			sameDir = false
		}
	}
	return
}

const AboutMessage = `https://flyingcarpet.spiegl.dev
Version: 6.0
theron@spiegl.dev
Copyright (c) 2022, Theron Spiegl. All rights reserved.
Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.
* Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.
* Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

Flying Carpet performs encrypted file transfers between two computers with wireless cards via ad hoc WiFi (or Wi-Fi Direct if necessary). No access point, router, or other networking gear is required. Just select a file, whether each computer is sending or receiving, and the operating system of the other computer. Flying Carpet will do its best to restore your wireless settings afterwards, but if there is an error, you may have to rejoin your wireless network manually. Thanks for using it and please provide feedback on GitHub!`
