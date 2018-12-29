package main

import (
	"github.com/therecipe/qt/widgets"
)

// Gui fulfills the UI interface to be used in the core functions
type Gui struct {
	// handles to progress bar, output box, start button
}

// Output prints messages to outputBox.
func (gui Gui) Output(msg string) {

	//for testing
	// file, err := os.OpenFile("err.txt", os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
	// if err != nil {
	// 	panic(err)
	// }
	// defer file.Close()
	// file.WriteString(msg)
	// file.WriteString("\r\n")
}

func newWindow() (window *widgets.QMainWindow) {
	// frame
	window = widgets.NewQMainWindow(nil, 0)
	window.SetMinimumSize2(600, 900)
	window.SetWindowTitle("Flying Carpet")
	widget := widgets.NewQWidget(nil, 0)
	widget.SetLayout(widgets.NewQVBoxLayout())
	window.SetCentralWidget(widget)

	// radio buttons
	radioWidget := widgets.NewQWidget(nil, 0)
	radioWidget.SetLayout(widgets.NewQHBoxLayout())

	peerWrapper := widgets.NewQGroupBox2("Peer OS", nil)
	peerLayout := widgets.NewQVBoxLayout2(peerWrapper)
	linuxPeer := widgets.NewQRadioButton2("Linux", nil)
	macPeer := widgets.NewQRadioButton2("Mac", nil)
	windowsPeer := widgets.NewQRadioButton2("Windows", nil)
	linuxPeer.SetChecked(true)
	peerLayout.AddWidget(linuxPeer, 0, 0)
	peerLayout.AddWidget(macPeer, 0, 0)
	peerLayout.AddWidget(windowsPeer, 0, 0)

	modeWrapper := widgets.NewQGroupBox2("Mode", nil)
	modeLayout := widgets.NewQVBoxLayout2(modeWrapper)
	sendMode := widgets.NewQRadioButton2("Send", nil)
	receiveMode := widgets.NewQRadioButton2("Receive", nil)
	sendMode.SetChecked(true)
	modeLayout.AddWidget(sendMode, 0, 0)
	modeLayout.AddWidget(receiveMode, 0, 0)

	radioWidget.Layout().AddWidget(peerWrapper)
	radioWidget.Layout().AddWidget(modeWrapper)

	// file box
	fileWidget := widgets.NewQWidget(nil, 0)
	fileWidget.SetLayout(widgets.NewQHBoxLayout())
	fileBox := widgets.NewQLineEdit(nil)
	sendButton := widgets.NewQPushButton2("Select file(s)", nil)
	receiveButton := widgets.NewQPushButton2("Select folder", nil)
	receiveButton.Hide()
	fileWidget.Layout().AddWidget(sendButton)
	fileWidget.Layout().AddWidget(receiveButton)
	fileWidget.Layout().AddWidget(fileBox)

	// start button
	startButton := widgets.NewQPushButton2("Start", nil)

	// output box
	outputBox := widgets.NewQTextEdit(nil)
	outputBox.SetSizePolicy2(widgets.QSizePolicy__Expanding, widgets.QSizePolicy__Expanding)
	outputBox.SetText("Welcome to Flying Carpet!\nInstructions:\n1. select the OS of the other device\n2. select whether this device is sending or receiving\n" +
		"3. select the files you'd like to send or the folder to which you'd like to receive\n4. press Start!")

	// progress bar
	progressBar := widgets.NewQProgressBar(nil)
	progressBar.Hide()
	// progressBar.SetValue(50)

	widget.Layout().AddWidget(radioWidget)
	widget.Layout().AddWidget(fileWidget)
	widget.Layout().AddWidget(startButton)
	widget.Layout().AddWidget(outputBox)
	widget.Layout().AddWidget(progressBar)

	//////////////////////////////
	/////////// ACTIONS //////////
	//////////////////////////////

	sendMode.ConnectClicked(func(bool) {
		sendButton.Show()
		receiveButton.Hide()
	})
	receiveMode.ConnectClicked(func(bool) {
		receiveButton.Show()
		sendButton.Hide()
	})

	sendButton.ConnectClicked(func(bool) {
		// open dialog
		fd := widgets.NewQFileDialog(window, 0)
		files := fd.GetOpenFileNames(window, "Select File(s)", "~", "", "", 0)
		if len(files) == 1 {
			fileBox.SetText(files[0])
		} else {
			fileBox.SetText("(Multiple files selected)")
		}

	})
	receiveButton.ConnectClicked(func(bool) {
		// open dialog
		fd := widgets.NewQFileDialog(window, 0)
		folder := fd.GetExistingDirectory(window, "Select Folder", "~", 0)
		fileBox.SetText(folder)
		// TODO: make sure contents of filebox is actually a folder before transfer
	})

	return
}
