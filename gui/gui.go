package main

import (
	"context"
	"os"
	"path/filepath"

	fccore "github.com/spieglt/flyingcarpet/core"
	"github.com/therecipe/qt/core"
	"github.com/therecipe/qt/widgets"
)

// Gui fulfills the UI interface to be used in the core functions
type Gui struct {
	ProgressBar  *widgets.QProgressBar
	OutputBox    *widgets.QTextEdit
	StartButton  *widgets.QPushButton
	CancelButton *widgets.QPushButton
}

// Output prints messages to outputBox.
func (gui Gui) Output(msg string) {
	gui.OutputBox.Append(msg)
}

// ShowProgressBar shows the progress bar when the transfer starts.
func (gui Gui) ShowProgressBar() {
	gui.ProgressBar.Show()
}

// UpdateProgressBar sets the percentage of the current file transferred.
func (gui Gui) UpdateProgressBar(percentDone int) {
	gui.ProgressBar.SetValue(percentDone)
}

// ToggleStartButton flips between the start and cancel buttons at the start
// and end of a transfer.
func (gui Gui) ToggleStartButton() {
	if gui.StartButton.IsHidden() {
		gui.CancelButton.Hide()
		gui.StartButton.Show()
		return
	}
	gui.CancelButton.Show()
	gui.StartButton.Hide()
}

func newWindow(gui *Gui) *widgets.QMainWindow {
	// frame
	window := widgets.NewQMainWindow(nil, 0)
	window.SetMinimumSize2(400, 600)
	window.SetWindowTitle("Flying Carpet")
	widget := widgets.NewQWidget(nil, 0)
	widget.SetLayout(widgets.NewQVBoxLayout())
	window.SetCentralWidget(widget)

	// radio buttons
	radioWidget := widgets.NewQWidget(nil, 0)
	radioWidget.SetLayout(widgets.NewQHBoxLayout())

	peerWrapper := widgets.NewQGroupBox2("Step 1: Select Peer OS", nil)
	peerLayout := widgets.NewQVBoxLayout2(peerWrapper)
	linuxPeer := widgets.NewQRadioButton2("Linux", nil)
	macPeer := widgets.NewQRadioButton2("Mac", nil)
	windowsPeer := widgets.NewQRadioButton2("Windows", nil)
	linuxPeer.SetChecked(true)
	peerLayout.AddWidget(linuxPeer, 0, 0)
	peerLayout.AddWidget(macPeer, 0, 0)
	peerLayout.AddWidget(windowsPeer, 0, 0)

	modeWrapper := widgets.NewQGroupBox2("Step 2: Select Mode", nil)
	modeLayout := widgets.NewQVBoxLayout2(modeWrapper)
	sendMode := widgets.NewQRadioButton2("Send", nil)
	receiveMode := widgets.NewQRadioButton2("Receive", nil)
	sendMode.SetChecked(true)
	modeLayout.AddWidget(sendMode, 0, 0)
	modeLayout.AddWidget(receiveMode, 0, 0)

	radioWidget.Layout().AddWidget(peerWrapper)
	radioWidget.Layout().AddWidget(modeWrapper)

	// file box
	fileWidget := widgets.NewQGroupBox2("Step 3: Select Files to Send or Destination Folder", nil)
	fileWidget.SetLayout(widgets.NewQHBoxLayout())
	sendButton := widgets.NewQPushButton2("Select file(s)", nil)
	receiveButton := widgets.NewQPushButton2("Select folder", nil)
	receiveButton.Hide()
	fileBox := widgets.NewQLineEdit(nil)
	fileBox.SetReadOnly(true)
	fileWidget.Layout().AddWidget(sendButton)
	fileWidget.Layout().AddWidget(receiveButton)
	fileWidget.Layout().AddWidget(fileBox)

	// start/cancel buttons
	startButton := widgets.NewQPushButton2("Start", nil)
	cancelButton := widgets.NewQPushButton2("Cancel", nil)
	cancelButton.Hide()

	// output box
	outputBox := widgets.NewQTextEdit(nil)
	outputBox.SetReadOnly(true)
	outputBox.SetSizePolicy2(widgets.QSizePolicy__Expanding, widgets.QSizePolicy__Expanding)
	outputBox.SetText("Welcome to Flying Carpet!\n")

	// progress bar
	progressBar := widgets.NewQProgressBar(nil)
	progressBar.Hide()

	widget.Layout().AddWidget(radioWidget)
	widget.Layout().AddWidget(fileWidget)
	widget.Layout().AddWidget(startButton)
	widget.Layout().AddWidget(cancelButton)
	widget.Layout().AddWidget(outputBox)
	widget.Layout().AddWidget(progressBar)

	// fill out gui with handles used to update UI from core
	gui = &Gui{
		ProgressBar:  progressBar,
		OutputBox:    outputBox,
		StartButton:  startButton,
		CancelButton: cancelButton,
	}

	//////////////////////////////
	/////////// ACTIONS //////////
	//////////////////////////////

	t := &fccore.Transfer{}

	sendMode.ConnectClicked(func(bool) {
		sendButton.Show()
		receiveButton.Hide()
		t.FileList = nil
		t.ReceiveDir = ""
		fileBox.SetText("")
	})
	receiveMode.ConnectClicked(func(bool) {
		receiveButton.Show()
		sendButton.Hide()
		t.FileList = nil
		t.ReceiveDir = ""
		fileBox.SetText("")
	})

	sendButton.ConnectClicked(func(bool) {
		// open dialog
		fd := widgets.NewQFileDialog(window, 0)
		t.FileList = fd.GetOpenFileNames(window, "Select File(s)", "", "", "", 0)
		if len(t.FileList) == 1 {
			fileBox.SetText(t.FileList[0])
		} else {
			fileBox.SetText("(Multiple files selected)")
		}
	})
	receiveButton.ConnectClicked(func(bool) {
		// open dialog
		fd := widgets.NewQFileDialog(window, 0)
		t.ReceiveDir = fd.GetExistingDirectory(window, "Select Folder", "", 0)
		fileBox.SetText(t.ReceiveDir)
	})

	startButton.ConnectClicked(func(bool) {
		switch {
		case sendMode.IsChecked():
			t.Mode = "sending"
		case receiveMode.IsChecked():
			t.Mode = "receiving"
		}
		switch {
		case linuxPeer.IsChecked():
			t.Peer = "linux"
		case macPeer.IsChecked():
			t.Peer = "mac"
		case windowsPeer.IsChecked():
			t.Peer = "windows"
		}
		// make sure something was selected
		if t.FileList == nil && t.ReceiveDir == "" {
			gui.Output("Error: please select files or a folder.")
			return
		}
		if t.Mode == "sending" {
			// make sure files exist
			for _, file := range t.FileList {
				_, err := os.Stat(file)
				if err != nil {
					gui.Output("Could not find output file " + file)
					gui.Output(err.Error())
					return
				}
			}
			// get password
			ok := false
			t.Password = widgets.QInputDialog_GetText(nil,
				"Enter Password", "Please start the transfer on the receiving end and enter the password that is displayed.",
				widgets.QLineEdit__Normal, "", &ok, core.Qt__Popup, core.Qt__ImhNone)
			if !ok || t.Password == "" {
				gui.Output("Transfer was canceled")
				return
			}
			if len(t.FileList) > 1 {
				gui.Output("Files selected:")
				for _, file := range t.FileList {
					gui.Output(file)
				}
			}
			gui.Output("Entered password: " + t.Password)
		} else if t.Mode == "receiving" {
			// make sure folder exists. necessary since fileBox is read-only?
			fpStat, err := os.Stat(t.ReceiveDir)
			if err != nil {
				gui.Output("Please select valid folder.")
				return
			}
			// make sure it ends with slash. also not necessary if fileBox is read-only.
			if !fpStat.IsDir() {
				t.ReceiveDir = filepath.Dir(t.ReceiveDir) + string(os.PathSeparator)
			}
			// show password
			t.Password = fccore.GeneratePassword()
			pwBox := widgets.NewQMessageBox(nil)
			pwBox.SetText("On sending end, after selecting options, press Start and enter this password:\n" + t.Password)
			// TODO: make this not block
			pwBox.Exec()
		}
		gui.ToggleStartButton()
		t.WfdSendChan, t.WfdRecvChan = make(chan string), make(chan string)
		t.Ctx, t.CancelCtx = context.WithCancel(context.Background())
		t.Port = 3290
		go fccore.StartTransfer(t, gui)
	})
	cancelButton.ConnectClicked(func(bool) {
		t.CancelCtx()
	})

	return window
}
