package main

import (
	"crypto/md5"
	"fmt"
	// "log"
	"math/rand"
	"net"
	"os"
	"runtime"
	"strconv"
	"strings"
	"time"
	"github.com/dontpanic92/wxGo/wx"
	"os/user"
)

// Change later
var OutputBox wx.TextCtrl
var StartButton wx.Button

const DIAL_TIMEOUT = 60
const JOIN_ADHOC_TIMEOUT = 60
const FIND_MAC_TIMEOUT = 60

func main() {
	wx1 := wx.NewApp()
	f := newGui()
	f.Show()
	wx1.MainLoop()
	return
}

func (t *Transfer) mainRoutine(mode string) {
	receiveChan := make(chan bool)
	sendChan := make(chan bool)
	var n Network

	if mode == "send" {
		pwBytes := md5.Sum([]byte(t.Passphrase))
		prefix := pwBytes[:3]
		t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)

		if runtime.GOOS == "windows" {
			w := WindowsNetwork{Mode: "sending"}
			w.PreviousSSID = w.getCurrentWifi()
			n = w
		} else if runtime.GOOS == "darwin" {
			n = MacNetwork{Mode: "sending"}
		}
		if !n.connectToPeer(t) {
			StartButton.Enable(true)
			OutputBox.AppendText("\nExiting mainRoutine.")
			return
		}

		if connected := t.sendFile(sendChan, n); connected == false {
			StartButton.Enable(true)
			OutputBox.AppendText("\nCould not establish TCP connection with peer. Exiting mainRoutine.")
			return
		}
		OutputBox.AppendText("\nConnected")
		sendSuccess := <-sendChan
		if !sendSuccess {
			StartButton.Enable(true)
			OutputBox.AppendText("\nExiting mainRoutine.")
			return
		}
		StartButton.Enable(true)
		OutputBox.AppendText("\nSend complete, resetting WiFi and exiting.")

	} else if mode == "receive" {
		t.Passphrase = generatePassword()
		pwBytes := md5.Sum([]byte(t.Passphrase))
		prefix := pwBytes[:3]
		t.SSID = fmt.Sprintf("flyingCarpet_%x", prefix)
		OutputBox.AppendText(fmt.Sprintf("\n=============================\n" +
			"Transfer password: %s\nPlease use this password on sending end when prompted to start transfer.\n" +
			"=============================\n",t.Passphrase))

		if runtime.GOOS == "windows" {
			n = WindowsNetwork{Mode: "receiving"}
		} else if runtime.GOOS == "darwin" {
			n = MacNetwork{Mode: "receiving"}
		}
		if !n.connectToPeer(t) {
			StartButton.Enable(true)
			OutputBox.AppendText("\nExiting mainRoutine.")
			return
		}

		go t.receiveFile(receiveChan, n)
		// wait for listener to be up
		listenerIsUp := <-receiveChan
		if !listenerIsUp {
			StartButton.Enable(true)
			OutputBox.AppendText("\nExiting mainRoutine.")
			return
		}
		// wait for reception to finish
		receiveSuccess := <-receiveChan
		if !receiveSuccess {
			StartButton.Enable(true)
			OutputBox.AppendText("\nExiting mainRoutine.")
			return
		}
		OutputBox.AppendText("\nReception complete, resetting WiFi and exiting.")
	}
	n.resetWifi(t)
}

func (t *Transfer) receiveFile(receiveChan chan bool, n Network) {
	ln, err := net.Listen("tcp", ":"+strconv.Itoa(t.Port))
	if err != nil {
		n.teardown(t)
		OutputBox.AppendText(fmt.Sprintf("\nCould not listen on :%d",t.Port))
		receiveChan <- false
		return
	}
	OutputBox.AppendText("\nListening on :" + strconv.Itoa(t.Port))
	receiveChan <- true
	for {
		conn, err := ln.Accept()
		if err != nil {
			n.teardown(t)
			OutputBox.AppendText(fmt.Sprintf("\nError accepting connection on :%d",t.Port))
			receiveChan <- false
			return
		}
		t.Conn = conn
		OutputBox.AppendText("\nConnection accepted")
		go t.receiveAndAssemble(receiveChan, n)
	}
}

func (t *Transfer) sendFile(sendChan chan bool, n Network) bool {
	var conn net.Conn
	var err error
	OutputBox.AppendText("\n")
	for i := 0; i < DIAL_TIMEOUT; i++ {
		err = nil
		conn, err = net.DialTimeout("tcp", t.RecipientIP+":"+strconv.Itoa(t.Port), time.Millisecond * 10)
		if err != nil {
			OutputBox.Replace(strings.LastIndex(OutputBox.GetValue(), "\n") + 1, OutputBox.GetLastPosition(), 
				fmt.Sprintf("\nFailed connection %2d to %s, retrying.", i, t.RecipientIP))
			time.Sleep(time.Second * 1)
			continue
		}
		OutputBox.AppendText("\nSuccessfully dialed peer.")
		t.Conn = conn
		go t.chunkAndSend(sendChan, n)
		return true
	}
	OutputBox.AppendText(fmt.Sprintf("Waited %d seconds, no connection.", DIAL_TIMEOUT))
	return false
}

func generatePassword() string {
	const chars = "0123456789abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ"
	rand.Seed(time.Now().UTC().UnixNano())
	pwBytes := make([]byte, 8)
	for i := range pwBytes {
		pwBytes[i] = chars[rand.Intn(len(chars))]
	}
	return string(pwBytes)
}

func newGui() *MainFrame {

	f := &MainFrame{}
	f.Frame = wx.NewFrame(wx.NullWindow, wx.ID_ANY, "Flying Carpet")
	
	f.SetSize(400,400)
	
	// radio buttons box
	radioSizer := wx.NewBoxSizer( wx.HORIZONTAL )

	// peer os box
	peerSizer := wx.NewBoxSizer( wx.VERTICAL )
	radiobox1 := wx.NewRadioBox( f, wx.ID_ANY, "Peer OS", wx.DefaultPosition, wx.DefaultSize, []string{"macOS", "Windows"}, 1, wx.HORIZONTAL )
	peerSizer.Add( radiobox1, 1, wx.ALL|wx.EXPAND, 5 )
	
	// bottom half and big container
	bSizerBottom := wx.NewBoxSizer( wx.VERTICAL )
	bSizerTotal := wx.NewBoxSizer( wx.VERTICAL )

	// file selection box
	fileSizer := wx.NewBoxSizer( wx.HORIZONTAL )
	sendButton := wx.NewButton(f, wx.ID_ANY, "Select File", wx.DefaultPosition, wx.DefaultSize, 0)
	receiveButton := wx.NewButton(f, wx.ID_ANY, "Select Folder", wx.DefaultPosition, wx.DefaultSize, 0)
	receiveButton.Hide()
	fileBox := wx.NewTextCtrl( f, wx.ID_ANY, "", wx.DefaultPosition, wx.DefaultSize, 0 )
	fileSizer.Add( sendButton, 0, wx.ALL|wx.EXPAND, 5 )
	fileSizer.Add( receiveButton, 0, wx.ALL|wx.EXPAND, 5 )
	fileSizer.Add( fileBox, 1, wx.ALL|wx.EXPAND, 5 )
	
	bSizerBottom.Add(fileSizer, 0, wx.ALL|wx.EXPAND, 5 )

	radioSizer.Add( peerSizer, 1, wx.EXPAND, 5 )
	modeSizer := wx.NewBoxSizer( wx.VERTICAL )

	radiobox2 := wx.NewRadioBox(f, wx.ID_ANY, "Mode", wx.DefaultPosition, wx.DefaultSize, []string{"Send", "Receive"}, 1, wx.HORIZONTAL )
	modeSizer.Add( radiobox2, 1, wx.ALL|wx.EXPAND, 5 )
	
	startButton := wx.NewButton( f, wx.ID_ANY, "Start", wx.DefaultPosition, wx.DefaultSize, 0)
	StartButton = startButton
	bSizerBottom.Add( startButton, 0, wx.ALL|wx.EXPAND, 5 )
	outputBox := wx.NewTextCtrl( f, wx.ID_ANY, "", wx.DefaultPosition, wx.DefaultSize, wx.TE_MULTILINE | wx.TE_READONLY )
	OutputBox = outputBox
	OutputBox.AppendText("Welcome to Flying Carpet!")

	bSizerBottom.Add( outputBox, 1, wx.ALL|wx.EXPAND, 5 )
	outputBox.SetSize(200,200);

	radioSizer.Add( modeSizer, 1, wx.EXPAND, 5 )

	bSizerTotal.Add( radioSizer, 0, wx.EXPAND, 5 )
	bSizerTotal.Add( bSizerBottom, 1, wx.EXPAND, 5 )

	// mode button action
	wx.Bind(f, wx.EVT_RADIOBOX, func(e wx.Event) {
		if radiobox2.GetSelection() == 0 {
			receiveButton.Hide()
			sendButton.Show()
		} else if radiobox2.GetSelection() == 1 {
			sendButton.Hide()
			receiveButton.Show()
		}
		f.Layout()
	}, radiobox2.GetId())

	// send button action
	wx.Bind(f, wx.EVT_BUTTON, func(e wx.Event) {
		fd := wx.NewFileDialogT(wx.NullWindow, "Select Files", "", "", "*", wx.FD_OPEN, wx.DefaultPosition, wx.DefaultSize, "Open")
		if fd.ShowModal() != wx.ID_CANCEL {
			filename := fd.GetPath()
			fileBox.SetValue(filename)
		}
	}, sendButton.GetId())

	// receive button action
	wx.Bind(f, wx.EVT_BUTTON, func(e wx.Event) {
		fd := wx.NewDirDialogT(wx.NullWindow, "Select Folder", "Open", wx.DD_DEFAULT_STYLE, wx.DefaultPosition, wx.DefaultSize)

		usr, err := user.Current()
		if err != nil {
			panic("Could not get user")
		}
		fd.SetPath(usr.HomeDir)

		if fd.ShowModal() != wx.ID_CANCEL {
			folder := fd.GetPath()
			fileBox.SetValue(folder + string(os.PathSeparator) + "file.out")
		}
	}, receiveButton.GetId())

	// start button action
	wx.Bind(f, wx.EVT_BUTTON, func(e wx.Event) {
		mode, peer := "", ""
		if radiobox2.GetSelection() == 0 {
			mode = "send"
		} else if radiobox2.GetSelection() == 1 {
			mode = "receive"
		}
		if radiobox1.GetSelection() == 0 {
			peer = "mac"
		} else if radiobox1.GetSelection() == 1 {
			peer = "windows"
		}

		t := Transfer{
			Filepath:   fileBox.GetValue(),
			Port:       3290,
			Peer:       peer,
			AdHocChan:	make(chan bool),
		}

		if mode == "send" {
			pd := wx.NewPasswordEntryDialog(f, "Enter password from receiving end:", "", "", wx.OK|wx.CANCEL, wx.DefaultPosition)
			ret := pd.ShowModal()
			if ret == wx.ID_OK {
				_,err := os.Stat(t.Filepath)
				if err == nil {
					startButton.Enable(false)
					outputBox.AppendText("\nEntered password: " + pd.GetValue())
					t.Passphrase = pd.GetValue()
					pd.Destroy()
					go t.mainRoutine(mode)
				} else {
					outputBox.AppendText("\nCould not find output file.")	
				}
			} else {
				outputBox.AppendText("\nPassword entry was cancelled.")
			}
		} else if mode == "receive" {
			_,err := os.Stat(t.Filepath)
			if err != nil {
				startButton.Enable(false)
				go t.mainRoutine(mode)	
			} else {
				outputBox.AppendText("\nError: destination file already exists.")
			}
		}
	}, startButton.GetId())
	
	f.SetSizer( bSizerTotal )
	f.Layout()
	f.Centre( wx.BOTH )
	
	return f

}

type Transfer struct {
	Filepath    string
	Passphrase  string
	SSID        string
	Conn        net.Conn
	Port        int
	RecipientIP string
	Peer        string
	AdHocChan   chan bool
}

type Network interface {
	connectToPeer(*Transfer) bool
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

type MainFrame struct {
	wx.Frame
	menuBar wx.MenuBar
}