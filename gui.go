package main

import (
	"github.com/dontpanic92/wxGo/wx"
	"os"
	"os/user"
)

const OUTPUT_BOX_UPDATE = wx.ID_HIGHEST + 1
const PROGRESS_BAR_UPDATE = wx.ID_HIGHEST + 2
const PROGRESS_BAR_SHOW = wx.ID_HIGHEST + 3
const START_BUTTON_ENABLE = wx.ID_HIGHEST + 4

type MainFrame struct {
	wx.Frame
	menuBar wx.MenuBar
}

func newGui() *MainFrame {
	mf := &MainFrame{}
	mf.Frame = wx.NewFrame(wx.NullWindow, wx.ID_ANY, "Flying Carpet")

	mf.SetSize(400, 400)

	// radio buttons box
	radioSizer := wx.NewBoxSizer(wx.HORIZONTAL)

	// peer os box
	peerSizer := wx.NewBoxSizer(wx.VERTICAL)
	radiobox1 := wx.NewRadioBox(mf, wx.ID_ANY, "Peer OS", wx.DefaultPosition, wx.DefaultSize, []string{"macOS", "Windows"}, 1, wx.HORIZONTAL)
	peerSizer.Add(radiobox1, 1, wx.ALL|wx.EXPAND, 5)

	// bottom half and big container
	bSizerBottom := wx.NewBoxSizer(wx.VERTICAL)
	bSizerTotal := wx.NewBoxSizer(wx.VERTICAL)

	// file selection box
	fileSizer := wx.NewBoxSizer(wx.HORIZONTAL)
	sendButton := wx.NewButton(mf, wx.ID_ANY, "Select File", wx.DefaultPosition, wx.DefaultSize, 0)
	receiveButton := wx.NewButton(mf, wx.ID_ANY, "Select Folder", wx.DefaultPosition, wx.DefaultSize, 0)
	receiveButton.Hide()
	fileBox := wx.NewTextCtrl(mf, wx.ID_ANY, "", wx.DefaultPosition, wx.DefaultSize, 0)
	fileSizer.Add(sendButton, 0, wx.ALL|wx.EXPAND, 5)
	fileSizer.Add(receiveButton, 0, wx.ALL|wx.EXPAND, 5)
	fileSizer.Add(fileBox, 1, wx.ALL|wx.EXPAND, 5)

	bSizerBottom.Add(fileSizer, 0, wx.ALL|wx.EXPAND, 5)

	radioSizer.Add(peerSizer, 1, wx.EXPAND, 5)
	modeSizer := wx.NewBoxSizer(wx.VERTICAL)

	radiobox2 := wx.NewRadioBox(mf, wx.ID_ANY, "Mode", wx.DefaultPosition, wx.DefaultSize, []string{"Send", "Receive"}, 1, wx.HORIZONTAL)
	modeSizer.Add(radiobox2, 1, wx.ALL|wx.EXPAND, 5)

	startButton := wx.NewButton(mf, wx.ID_ANY, "Start", wx.DefaultPosition, wx.DefaultSize, 0)
	bSizerBottom.Add(startButton, 0, wx.ALL|wx.EXPAND, 5)
	outputBox := wx.NewTextCtrl(mf, wx.ID_ANY, "", wx.DefaultPosition, wx.DefaultSize, wx.TE_MULTILINE|wx.TE_READONLY)
	outputBox.AppendText("Welcome to Flying Carpet!\n")

	progressBar := wx.NewGauge(mf, wx.ID_ANY, 100, wx.DefaultPosition, wx.DefaultSize, wx.GA_HORIZONTAL)
	progressBar.Hide()

	bSizerBottom.Add(outputBox, 1, wx.ALL|wx.EXPAND, 0)
	outputBox.SetSize(200, 200)
	bSizerBottom.Add(progressBar, 0, wx.ALL|wx.EXPAND, 5)

	radioSizer.Add(modeSizer, 1, wx.EXPAND, 5)

	bSizerTotal.Add(radioSizer, 0, wx.EXPAND, 5)
	bSizerTotal.Add(bSizerBottom, 1, wx.EXPAND, 5)

	// mode button action
	wx.Bind(mf, wx.EVT_RADIOBOX, func(e wx.Event) {
		if radiobox2.GetSelection() == 0 {
			receiveButton.Hide()
			sendButton.Show()
		} else if radiobox2.GetSelection() == 1 {
			sendButton.Hide()
			receiveButton.Show()
		}
		mf.Layout()
	}, radiobox2.GetId())

	// send button action
	wx.Bind(mf, wx.EVT_BUTTON, func(e wx.Event) {
		fd := wx.NewFileDialogT(wx.NullWindow, "Select Files", "", "", "*", wx.FD_OPEN, wx.DefaultPosition, wx.DefaultSize, "Open")
		if fd.ShowModal() != wx.ID_CANCEL {
			filename := fd.GetPath()
			fileBox.SetValue(filename)
		}
	}, sendButton.GetId())

	// receive button action
	wx.Bind(mf, wx.EVT_BUTTON, func(e wx.Event) {
		fd := wx.NewDirDialogT(wx.NullWindow, "Select Folder", "Open", wx.DD_DEFAULT_STYLE, wx.DefaultPosition, wx.DefaultSize)

		usr, err := user.Current()
		if err != nil {
			panic("Could not get user")
		}
		fd.SetPath(usr.HomeDir + string(os.PathSeparator) + "Desktop")

		if fd.ShowModal() != wx.ID_CANCEL {
			folder := fd.GetPath()
			fileBox.SetValue(folder + string(os.PathSeparator) + "file.out")
		}
	}, receiveButton.GetId())

	// start button action
	wx.Bind(mf, wx.EVT_BUTTON, func(e wx.Event) {
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
			Filepath:  fileBox.GetValue(),
			Port:      3290,
			Peer:      peer,
			AdHocChan: make(chan bool),
			Frame:     mf,
		}

		if mode == "send" {
			pd := wx.NewPasswordEntryDialog(mf, "Enter password from receiving end:", "", "", wx.OK|wx.CANCEL, wx.DefaultPosition)
			ret := pd.ShowModal()
			if ret == wx.ID_OK {
				_, err := os.Stat(t.Filepath)
				if err == nil {
					startButton.Enable(false)
					outputBox.AppendText("Entered password: " + pd.GetValue())
					t.Passphrase = pd.GetValue()
					// pd.Destroy()
					go t.mainRoutine(mode)
				} else {
					outputBox.AppendText("Could not find output file.")
				}
			} else {
				outputBox.AppendText("Password entry was cancelled.")
			}
		} else if mode == "receive" {
			_, err := os.Stat(t.Filepath)
			if err != nil {
				startButton.Enable(false)
				go t.mainRoutine(mode)
			} else {
				outputBox.AppendText("Error: destination file already exists.")
			}
		}
	}, startButton.GetId())

	// output box update event
	wx.Bind(mf, wx.EVT_THREAD, func(e wx.Event) {
		threadEvent := wx.ToThreadEvent(e)
		outputBox.AppendText("\n" + threadEvent.GetString())
	}, OUTPUT_BOX_UPDATE)

	// progress bar update event
	wx.Bind(mf, wx.EVT_THREAD, func(e wx.Event) {
		threadEvent := wx.ToThreadEvent(e)
		progressBar.SetValue(threadEvent.GetInt())
	}, PROGRESS_BAR_UPDATE)

	// progress bar display event
	wx.Bind(mf, wx.EVT_THREAD, func(e wx.Event) {
		progressBar.Show()
		mf.Layout()
	}, PROGRESS_BAR_SHOW)

	// start button enable event
	wx.Bind(mf, wx.EVT_THREAD, func(e wx.Event) {
		startButton.Enable(true)
		mf.Layout()
	}, START_BUTTON_ENABLE)

	mf.SetSizer(bSizerTotal)
	mf.Layout()
	mf.Centre(wx.BOTH)

	return mf
}

func (t *Transfer) output(msg string) {
	threadEvt := wx.NewThreadEvent(wx.EVT_THREAD, OUTPUT_BOX_UPDATE)
	threadEvt.SetString(msg)
	t.Frame.QueueEvent(threadEvt)
}

func (t *Transfer) enableStartButton() {
	startButtonEvt := wx.NewThreadEvent(wx.EVT_THREAD, START_BUTTON_ENABLE)
	t.Frame.QueueEvent(startButtonEvt)
}
