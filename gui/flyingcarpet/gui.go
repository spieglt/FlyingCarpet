package main

import (
	"context"
	"os"

	fcc "github.com/spieglt/flyingcarpet/core"
	"github.com/therecipe/qt/core"
	"github.com/therecipe/qt/widgets"
)

// Gui fulfills the UI interface to be used in the core functions
type Gui struct {
	ProgressBar  *widgets.QProgressBar
	OutputBox    *widgets.QTextEdit
	StartButton  *widgets.QPushButton
	CancelButton *widgets.QPushButton
	// for password prompt dialog on Mac
	PromptAction *widgets.QAction
	PromptChan   *chan bool
}

// Output prints messages to outputBox.
func (gui *Gui) Output(msg string) {
	gui.OutputBox.Append(msg)
}

// ShowProgressBar shows the progress bar when the transfer starts.
func (gui *Gui) ShowProgressBar() {
	gui.ProgressBar.Show()
}

// UpdateProgressBar sets the percentage of the current file transferred.
func (gui *Gui) UpdateProgressBar(percentDone int) {
	gui.ProgressBar.SetValue(percentDone)
}

// ToggleStartButton flips between the start and cancel buttons at the start
// and end of a transfer.
func (gui *Gui) ToggleStartButton() {
	if gui.StartButton.IsHidden() {
		gui.CancelButton.Hide()
		gui.StartButton.Show()
		return
	}
	gui.CancelButton.Show()
	gui.StartButton.Hide()
}

// ShowPwPrompt is only used on Mac after a transfer to prompt whether the user wants to enter
// their password to remove the Flying Carpet wireless network from their list of preferred networks.
func (gui *Gui) ShowPwPrompt() bool {
	gui.PromptAction.Trigger()
	return <-*(gui.PromptChan)
}

func newWindow(gui *Gui) *widgets.QMainWindow {
	// frame
	window := widgets.NewQMainWindow(nil, 0)
	window.SetMinimumSize2(400, 600)
	window.SetWindowTitle("Flying Carpet")
	widget := widgets.NewQWidget(nil, 0)
	widget.SetLayout(widgets.NewQVBoxLayout())
	window.SetCentralWidget(widget)

	// about menu
	fileMenu := window.MenuBar().AddMenu2("&About")
	aboutAction := widgets.NewQAction2("&About", window.MenuBar())
	aboutAction.ConnectTriggered(func(bool) { aboutBox() })
	fileMenu.AddActions([]*widgets.QAction{aboutAction})

	// radio buttons
	radioWidget := widgets.NewQWidget(nil, 0)
	radioWidget.SetLayout(widgets.NewQHBoxLayout())

	peerWrapper := widgets.NewQGroupBox2("Step 1: Select Peer OS", nil)
	peerLayout := widgets.NewQVBoxLayout2(peerWrapper)
	linuxPeer := widgets.NewQRadioButton2("Linux", nil)
	macPeer := widgets.NewQRadioButton2("Mac", nil)
	windowsPeer := widgets.NewQRadioButton2("Windows", nil)
	// linuxPeer.SetChecked(true)
	peerLayout.AddWidget(linuxPeer, 0, 0)
	peerLayout.AddWidget(macPeer, 0, 0)
	peerLayout.AddWidget(windowsPeer, 0, 0)

	modeWrapper := widgets.NewQGroupBox2("Step 2: Select Mode", nil)
	modeLayout := widgets.NewQVBoxLayout2(modeWrapper)
	sendMode := widgets.NewQRadioButton2("Send", nil)
	receiveMode := widgets.NewQRadioButton2("Receive", nil)
	// sendMode.SetChecked(true)
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
	outputBox.SetText("Welcome to Flying Carpet!")

	// progress bar
	progressBar := widgets.NewQProgressBar(nil)
	progressBar.Hide()

	// password prompt box
	promptBox := widgets.NewQMessageBox(nil)
	promptChan := make(chan bool)
	promptAction := widgets.NewQAction(nil)
	promptAction.ConnectTriggered(func(bool) {
		// password prompt box
		answer := promptBox.Question(nil, "Remove wireless network?",
			"Would you like Flying Carpet to remove itself from your preferred networks list? Click Yes to enter your password or No to skip. You can do this yourself later from the System Preferences menu.",
			widgets.QMessageBox__No|widgets.QMessageBox__Yes, widgets.QMessageBox__No)
		promptChan <- answer == widgets.QMessageBox__Yes
	})

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
		PromptAction: promptAction,
		PromptChan:   &promptChan,
	}

	//////////////////////////////
	/////////// ACTIONS //////////
	//////////////////////////////

	t := &fcc.Transfer{}

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
		t.ReceiveDir = getHomePath()
		fileBox.SetText(getHomePath())
	})

	sendButton.ConnectClicked(func(bool) {
		// open dialog
		fd := widgets.NewQFileDialog2(window, "Select Files", getHomePath(), "")
		t.FileList = fd.GetOpenFileNames(window, "Select File(s)", "", "", "", 0)
		if len(t.FileList) == 1 {
			fileBox.SetText(t.FileList[0])
		} else {
			fileBox.SetText("(Multiple files selected)")
		}
	})
	receiveButton.ConnectClicked(func(bool) {
		// open dialog
		fd := widgets.NewQFileDialog2(window, "Select Files", getHomePath(), "")
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
			if err != nil || !fpStat.IsDir() {
				gui.Output("Please select valid folder.")
				return
			}
			// make sure it ends with slash
			if t.ReceiveDir[len(t.ReceiveDir)-1] != os.PathSeparator {
				t.ReceiveDir += string(os.PathSeparator)
			}
			// show password
			t.Password = fcc.GeneratePassword()
			pwBox := widgets.NewQMessageBox(nil)
			pwBox.SetText("On sending end, after selecting options, press Start and enter this password:\n\n" + t.Password)
			pwBox.Show()
		}
		gui.ToggleStartButton()
		t.WfdSendChan, t.WfdRecvChan = make(chan string), make(chan string)
		t.Ctx, t.CancelCtx = context.WithCancel(context.Background())
		t.Port = 3290
		t.DllLocation = ".\\wfd.dll"
		go fcc.StartTransfer(t, gui)
	})
	cancelButton.ConnectClicked(func(bool) {
		t.CancelCtx()
	})

	return window
}

func adminCheck(gui *Gui) {
	if fcc.HostOS == "windows" {
		inGroup := fcc.IsUserInAdminGroup()
		isAdmin := fcc.IsRunAsAdmin()
		mb := widgets.NewQMessageBox(nil)
		if isAdmin == 0 {
			switch inGroup {
			case 0:
				mb.SetText("Flying Carpet needs admin privileges to create/delete a firewall rule, listen on a TCP port, and clear your ARP cache. Please run with an administrator account.")
				mb.Exec()
				os.Exit(5)
			case 1:
				mb.SetText("Flying Carpet needs admin privileges to create/delete a firewall rule, listen on a TCP port, and clear your ARP cache. Please click yes at the prompt to \"Run as administrator\" or no to exit.")
				mb.Exec()
				fcc.RelaunchAsAdmin()
				os.Exit(0)
			case 2:
				gui.Output("Error determining if current user is admin.")
			}
			os.Exit(5)
		} else {
			// TODO: why doesn't this print?
			gui.Output("We're admin!")
		}
	}
}

func aboutBox() {
	widgets.QMessageBox_About(nil, "About Flying Carpet", fcc.AboutMessage)
}

func getHomePath() (homePath string) {
	if fcc.HostOS == "windows" {
		homePath = os.Getenv("USERPROFILE")
	} else {
		homePath = os.Getenv("HOME")
	}
	return
}
