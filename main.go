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
	"github.com/dontpanic92/wxGo/wx"
	"os/user"
)

const DIAL_TIMEOUT = 60
const JOIN_ADHOC_TIMEOUT = 60
const FIND_MAC_TIMEOUT = 60

func main() {
	
	wx1 := wx.NewApp()
	f := newGui()
	f.Show()
	wx1.MainLoop()
	return

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

func newGui() *MainFrame {

	f := &MainFrame{}
	f.Frame = wx.NewFrame(wx.NullWindow, wx.ID_ANY, "Flying Carpet")
	
	f.SetSize(400,400)
	
	// radio buttons box
	radioSizer := wx.NewBoxSizer( wx.HORIZONTAL )

	// peer os box
	peerSizer := wx.NewBoxSizer( wx.VERTICAL )
	radiobox1 := wx.NewRadioBox( f, wx.ID_ANY, "Peer OS", wx.DefaultPosition, wx.DefaultSize, []string{"macOS", "Windows"}, 1, wx.HORIZONTAL )
	// radiobox1.SetSelection( 0 )
	peerSizer.Add( radiobox1, 1, wx.ALL|wx.EXPAND, 5 )
	
	// bottom half and big container
	bSizerBottom := wx.NewBoxSizer( wx.VERTICAL )
	// bSizerBottom.SetMinSize(200,200)
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
	bSizerBottom.Add( startButton, 0, wx.ALL|wx.EXPAND, 5 )
	txt := "here's a line\nand another\nand another\nand another\nand another\nand another\nand another\nand another\nand another\nand another\nand another\nand another"
	outputBox := wx.NewTextCtrl( f, wx.ID_ANY, txt, wx.DefaultPosition, wx.DefaultSize, wx.TE_MULTILINE | wx.TE_READONLY )
	
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
			log.Fatal( err )
		}
		fd.SetPath(usr.HomeDir)

		if fd.ShowModal() != wx.ID_CANCEL {
			folder := fd.GetPath()
			fileBox.SetValue(folder + string(os.PathSeparator) + "file.out")
		}
	}, receiveButton.GetId())

	// start button action
	wx.Bind(f, wx.EVT_BUTTON, func(e wx.Event) {
		pd := newPasswordDialog()
		pd.Show()
		// pd.Destroy()
	}, startButton.GetId())
	
	f.SetSizer( bSizerTotal )
	f.Layout()
	
	f.Centre( wx.BOTH )
	
	return f

}

func newPasswordDialog() *PasswordDialog {
	pd := &PasswordDialog{}
	pd.Dialog = wx.NewDialog()
	pd.SetSizeHints( wx.DefaultSize, wx.DefaultSize )
	total := wx.NewBoxSizer(wx.VERTICAL)
	pwBox := wx.NewTextCtrl( pd, wx.ID_ANY, "", wx.DefaultPosition, wx.DefaultSize, 0)
	submitButton := wx.NewButton(pd, wx.ID_ANY, "Submit", wx.DefaultPosition, wx.DefaultSize, 0)
	total.Add(pwBox, 0, wx.ALL|wx.EXPAND, 5)
	total.Add(submitButton, 0, wx.ALL|wx.EXPAND, 5)

	pd.SetSizer(total)
	pd.Layout()
	pd.Centre(wx.BOTH)
	pd.Show()
	pd.Destroy()
	return pd
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

type PasswordDialog struct {
	wx.Dialog
}

type MainFrame struct {
	wx.Frame
	menuBar wx.MenuBar
}