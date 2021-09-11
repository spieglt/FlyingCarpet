package main

import (
	"context"
	"fmt"
	"os"

	fcc "github.com/spieglt/flyingcarpet/core"
	"github.com/therecipe/qt/core"
	qgui "github.com/therecipe/qt/gui"
	"github.com/therecipe/qt/widgets"
)

// Gui fulfills the UI interface to be used in the core functions
type Gui struct {
	ProgressBar  *widgets.QProgressBar
	OutputBox    *widgets.QTextEdit
	StartButton  *widgets.QPushButton
	CancelButton *widgets.QPushButton
	FileBox      *widgets.QLineEdit
	// for password prompt dialog on Mac
	PromptAction *widgets.QAction
	PromptChan   *chan bool

	SendMode       *widgets.QRadioButton
	ReceiveMode    *widgets.QRadioButton
	LinuxPeer      *widgets.QRadioButton
	MacPeer        *widgets.QRadioButton
	WindowsPeer    *widgets.QRadioButton
	SendButton     *widgets.QPushButton
	ReceiveButton  *widgets.QPushButton
	sendChecked    bool
	receiveChecked bool
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
	enabled := false
	if gui.StartButton.IsHidden() {
		gui.CancelButton.Hide()
		gui.StartButton.Show()
		enabled = true
	} else {
		gui.CancelButton.Show()
		gui.StartButton.Hide()
	}
	gui.SendMode.SetEnabled(enabled)
	gui.ReceiveMode.SetEnabled(enabled)
	gui.LinuxPeer.SetEnabled(enabled)
	gui.MacPeer.SetEnabled(enabled)
	gui.WindowsPeer.SetEnabled(enabled)
	gui.SendButton.SetEnabled(enabled)
	gui.ReceiveButton.SetEnabled(enabled)
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

	// folder send toggle
	folderSendAction := widgets.NewQAction2("&Send Folder", window.MenuBar())
	folderSendAction.SetCheckable(true)
	folderSendAction.SetShortcut(qgui.QKeySequence_FromString("Ctrl+S", 0))

	// about menu
	fileMenu := window.MenuBar().AddMenu2("&Menu")
	aboutAction := widgets.NewQAction2("&About", window.MenuBar())
	aboutAction.ConnectTriggered(func(bool) { aboutBox() })

	// quit shortcut
	quitAction := widgets.NewQAction2("&Quit", window.MenuBar())
	quitAction.ConnectTriggered(func(bool) { window.Close() })
	quitAction.SetShortcut(qgui.QKeySequence_FromString("Ctrl+Q", 0))

	fileMenu.AddActions([]*widgets.QAction{
		folderSendAction,
		aboutAction,
		quitAction,
	})

	// radio buttons
	radioWidget := widgets.NewQWidget(nil, 0)
	radioWidget.SetLayout(widgets.NewQHBoxLayout())

	peerWrapper := widgets.NewQGroupBox2("Step 1: Select Peer OS", nil)
	peerLayout := widgets.NewQVBoxLayout2(peerWrapper)
	linuxPeer := widgets.NewQRadioButton2("Linux", nil)
	macPeer := widgets.NewQRadioButton2("Mac", nil)
	windowsPeer := widgets.NewQRadioButton2("Windows", nil)
	peerLayout.AddWidget(linuxPeer, 0, 0)
	peerLayout.AddWidget(macPeer, 0, 0)
	peerLayout.AddWidget(windowsPeer, 0, 0)

	modeWrapper := widgets.NewQGroupBox2("Step 2: Select Mode", nil)
	modeLayout := widgets.NewQVBoxLayout2(modeWrapper)
	sendMode := widgets.NewQRadioButton2("Send", nil)
	receiveMode := widgets.NewQRadioButton2("Receive", nil)
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
		FileBox:      fileBox,
		PromptAction: promptAction,
		PromptChan:   &promptChan,

		SendMode:      sendMode,
		ReceiveMode:   receiveMode,
		LinuxPeer:     linuxPeer,
		MacPeer:       macPeer,
		WindowsPeer:   windowsPeer,
		SendButton:    sendButton,
		ReceiveButton: receiveButton,
	}

	//////////////////////////////
	/////////// ACTIONS //////////
	//////////////////////////////

	t := &fcc.Transfer{}

	setUpDragAndDrop(widget, gui, t)

	folderSendAction.ConnectToggled(func(checked bool) {
		if checked {
			sendButton.SetText("Select folder")
		} else {
			sendButton.SetText("Select file(s)")
		}
	})

	sendMode.ConnectClicked(func(bool) {
		sendButton.Show()
		receiveButton.Hide()
		if !gui.sendChecked {
			t.FileList = nil
			t.ReceiveDir = ""
			fileBox.SetText("")
		}
		gui.sendChecked = true
		gui.receiveChecked = false
	})
	receiveMode.ConnectClicked(func(bool) {
		receiveButton.Show()
		sendButton.Hide()
		if !gui.receiveChecked {
			t.FileList = nil
			t.ReceiveDir = getHomePath()
			fileBox.SetText(getHomePath())
		}
		gui.sendChecked = false
		gui.receiveChecked = true
	})

	sendButton.ConnectClicked(func(bool) {
		if !sendMode.IsChecked() {
			gui.Output("Error: please select whether this device is sending or receiving.")
			return
		}
		// open dialog
		fd := widgets.NewQFileDialog2(window, "Select Files", getHomePath(), "")
		if !folderSendAction.IsChecked() {
			t.FileList = fd.GetOpenFileNames(window, "Select File(s)", "", "", "", 0)
		} else {
			directory := fd.GetExistingDirectory(window, "Select Folder", "", 0)
			t.FileList = []string{directory}
		}
		if len(t.FileList) == 1 {
			fileBox.SetText(t.FileList[0])
		} else if len(t.FileList) > 1 {
			fileBox.SetText("(Multiple files selected)")
			gui.Output("Files selected:")
			for _, file := range t.FileList {
				gui.Output(file)
			}
		}
	})
	receiveButton.ConnectClicked(func(bool) {
		// open dialog
		fd := widgets.NewQFileDialog2(window, "Select Folder", getHomePath(), "")
		t.ReceiveDir = fd.GetExistingDirectory(window, "Select Folder", "", 0)
		fileBox.SetText(t.ReceiveDir)
	})

	startButton.ConnectClicked(func(bool) {
		switch {
		case linuxPeer.IsChecked():
			t.Peer = "linux"
		case macPeer.IsChecked():
			t.Peer = "mac"
		case windowsPeer.IsChecked():
			t.Peer = "windows"
		default:
			gui.Output("Error: please select the operating system of the other device.")
			return
		}
		switch {
		case sendMode.IsChecked():
			t.Mode = "sending"
		case receiveMode.IsChecked():
			t.Mode = "receiving"
		default:
			gui.Output("Error: please select whether this device is sending or receiving.")
			return
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

func setUpDragAndDrop(widget *widgets.QWidget, gui *Gui, t *fcc.Transfer) {
	widget.SetAcceptDrops(true)
	widget.ConnectDropEvent(func(event *qgui.QDropEvent) {
		md := event.MimeData()
		// if md.HasText() {
		// 	fmt.Printf("event text: %s\n", md.Text())
		// }
		if md.HasUrls() {
			urls := md.Urls()
			// for _, url := range urls {
			// 	fmt.Printf("path: %s\n", url.Path(0))
			// 	fmt.Printf("host: %s\n", url.Host(0))
			// 	fmt.Printf("full: %s\n", url.ToDisplayString(0))
			// }
			switch {
			case gui.SendMode.IsChecked():
				fileList := make([]string, 0)
				for _, url := range urls {
					p := url.Path(0)
					_, err := os.Stat(p)
					if err != nil {
						gui.OutputBox.Append(fmt.Sprintf("Invalid file selected: %s. Error: %s.", p, err.Error()))
						return
						// } else if file.IsDir() {
						// 	gui.OutputBox.Append(fmt.Sprintf("Error: must select files only when sending. Directory: %s.", p))
						// 	return
					} else {
						fileList = append(fileList, p)
					}
				}
				if len(fileList) == 1 {
					gui.FileBox.SetText(fileList[0])
				} else if len(fileList) > 1 {
					gui.FileBox.SetText("(Multiple files selected)")
					gui.Output("Files selected:")
					for _, file := range t.FileList {
						gui.Output(file)
					}
				}
				t.FileList = fileList
			case gui.ReceiveMode.IsChecked():
				if len(urls) > 1 {
					gui.OutputBox.Append("Must select only one folder when receiving.")
					return
				}
				p := urls[0].Path(0)
				file, err := os.Stat(p)
				if err != nil {
					gui.OutputBox.Append(fmt.Sprintf("Invalid folder selected: %s. Error: %s.", p, err.Error()))
					return
				} else if !file.IsDir() {
					gui.OutputBox.Append(fmt.Sprintf("Error: must select folder when receiving: File: %s.", p))
					return
				}
				t.ReceiveDir = p
				gui.FileBox.SetText(p)
			default:
				gui.OutputBox.Append("Please select Send or Receive first.")
			}
		}
		event.AcceptProposedAction()
	})
	widget.ConnectDragEnterEvent(func(e *qgui.QDragEnterEvent) { e.AcceptProposedAction() })
	widget.ConnectDragMoveEvent(func(e *qgui.QDragMoveEvent) { e.AcceptProposedAction() })
	widget.ConnectDragLeaveEvent(func(e *qgui.QDragLeaveEvent) { e.Accept() })
}
