package main

import (
	"github.com/dontpanic92/wxGo/wx"
	"os"
	"os/user"
	"runtime"
)

const OUTPUT_BOX_UPDATE = wx.ID_HIGHEST + 1
const PROGRESS_BAR_UPDATE = wx.ID_HIGHEST + 2
const PROGRESS_BAR_SHOW = wx.ID_HIGHEST + 3
const START_BUTTON_ENABLE = wx.ID_HIGHEST + 4
const HIDE_OPTION_ID = wx.ID_HIGHEST + 5

type MainFrame struct {
	wx.Frame
	MenuBar wx.MenuBar
	Panel   wx.Panel
}

func newGui() *MainFrame {
	mf := &MainFrame{}
	mf.Frame = wx.NewFrame(wx.NullWindow, wx.ID_ANY, "Flying Carpet")


	// window

	mf.SetSize(400, 400)
	mf.Panel = wx.NewPanel(mf)
	mf.Panel.SetSize(400, 400)

	// big sizer
	bSizerTotal := wx.NewBoxSizer(wx.VERTICAL)

	// radio buttons box
	radioSizer := wx.NewBoxSizer(wx.HORIZONTAL)
	peerSizer := wx.NewBoxSizer(wx.VERTICAL)
	radiobox1 := wx.NewRadioBox(mf.Panel, wx.ID_ANY, "Peer OS", wx.DefaultPosition, wx.DefaultSize, []string{"macOS", "Windows"}, 1, wx.HORIZONTAL)
	peerSizer.Add(radiobox1, 1, wx.ALL|wx.EXPAND, 5)
	radioSizer.Add(peerSizer, 1, wx.EXPAND, 5)
	modeSizer := wx.NewBoxSizer(wx.VERTICAL)
	radiobox2 := wx.NewRadioBox(mf.Panel, wx.ID_ANY, "Mode", wx.DefaultPosition, wx.DefaultSize, []string{"Send", "Receive"}, 1, wx.HORIZONTAL)
	modeSizer.Add(radiobox2, 1, wx.ALL|wx.EXPAND, 5)
	radioSizer.Add(modeSizer, 1, wx.EXPAND, 5)

	// bottom half
	bSizerBottom := wx.NewBoxSizer(wx.VERTICAL)

	// file selection box
	fileSizer := wx.NewBoxSizer(wx.HORIZONTAL)
	sendButton := wx.NewButton(mf.Panel, wx.ID_ANY, "Select File", wx.DefaultPosition, wx.DefaultSize, 0)
	receiveButton := wx.NewButton(mf.Panel, wx.ID_ANY, "Select Folder", wx.DefaultPosition, wx.DefaultSize, 0)
	receiveButton.Hide()
	fileBox := wx.NewTextCtrl(mf.Panel, wx.ID_ANY, "", wx.DefaultPosition, wx.DefaultSize, 0)
	fileSizer.Add(sendButton, 0, wx.ALL|wx.EXPAND, 5)
	fileSizer.Add(receiveButton, 0, wx.ALL|wx.EXPAND, 5)
	fileSizer.Add(fileBox, 1, wx.ALL|wx.EXPAND, 5)
	bSizerBottom.Add(fileSizer, 0, wx.ALL|wx.EXPAND, 5)

	// start button
	startButton := wx.NewButton(mf.Panel, wx.ID_ANY, "Start", wx.DefaultPosition, wx.DefaultSize, 0)
	bSizerBottom.Add(startButton, 0, wx.ALL|wx.EXPAND, 5)

	// output box
	outputBox := wx.NewTextCtrl(mf.Panel, wx.ID_ANY, "", wx.DefaultPosition, wx.DefaultSize, wx.TE_MULTILINE|wx.TE_READONLY)
	outputBox.AppendText("Welcome to Flying Carpet!\n")
	bSizerBottom.Add(outputBox, 1, wx.ALL|wx.EXPAND, 0)
	outputBox.SetSize(200, 200)

	// progress bar
	progressBar := wx.NewGauge(mf.Panel, wx.ID_ANY, 100, wx.DefaultPosition, wx.DefaultSize, wx.GA_HORIZONTAL)
	progressBar.Hide()
	bSizerBottom.Add(progressBar, 0, wx.ALL|wx.EXPAND, 5)

	// stack top and bottom halves
	bSizerTotal.Add(radioSizer, 0, wx.EXPAND, 5)
	bSizerTotal.Add(bSizerBottom, 1, wx.EXPAND, 5)

	//////////////////////////////
	/////////// ACTIONS //////////
	//////////////////////////////

	// mode button action
	wx.Bind(mf, wx.EVT_RADIOBOX, func(e wx.Event) {
		if radiobox2.GetSelection() == 0 {
			receiveButton.Hide()
			sendButton.Show()
			fileBox.SetValue("")
		} else if radiobox2.GetSelection() == 1 {
			sendButton.Hide()
			receiveButton.Show()
			usr, _ := user.Current()
			fileBox.SetValue(usr.HomeDir + string(os.PathSeparator) + "Desktop" + string(os.PathSeparator) + "file.out")
		}
		mf.Panel.Layout()
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
			pd := wx.NewPasswordEntryDialog(mf.Panel, "Enter password from receiving end:", "", "", wx.OK|wx.CANCEL, wx.DefaultPosition)
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
		mf.Panel.Layout()
	}, PROGRESS_BAR_SHOW)

	// start button enable event
	wx.Bind(mf, wx.EVT_THREAD, func(e wx.Event) {
		startButton.Enable(true)
		mf.Panel.Layout()
	}, START_BUTTON_ENABLE)

	mf.Panel.SetSizer(bSizerTotal)
	mf.Layout()
	mf.Centre(wx.BOTH)


	// menu

	mf.MenuBar = wx.NewMenuBar()
	if runtime.GOOS == "windows" {
		fileMenu := wx.NewMenu()
		fileMenu.Append(wx.ID_ABOUT)
		fileMenu.Append(wx.ID_EXIT)
		wx.Bind(mf, wx.EVT_MENU, func(e wx.Event){
			mf.Close(true)
		}, wx.ID_EXIT)
		mf.MenuBar.Append(fileMenu, "&File")
	} else if runtime.GOOS == "darwin" {
		addAboutToOSXMenu()
	}
	wx.Bind(mf, wx.EVT_MENU, func(e wx.Event) {
		info := wx.NewAboutDialogInfo()
		info.SetName("Flying Carpet")
		info.SetDescription(DESCRIPTION)
		info.SetCopyright(COPYRIGHT)
		info.SetWebSite(WEBSITE)
		info.SetLicence(LICENSE)
		wx.AboutBox(info)
	}, wx.ID_ABOUT)
	mf.SetMenuBar(mf.MenuBar)

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

const WEBSITE = "https://github.com/spieglt/flyingcarpet"
const COPYRIGHT = "Copyright (c) 2017, Theron Spiegl. All rights reserved."
const LICENSE = 
`Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright notice, this
  list of conditions and the following disclaimer.

* Redistributions in binary form must reproduce the above copyright notice,
  this list of conditions and the following disclaimer in the documentation
  and/or other materials provided with the distribution.

* Neither the name of the copyright holder nor the names of its
  contributors may be used to endorse or promote products derived from
  this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.`
const DESCRIPTION = 
`Flying Carpet performs encrypted file transfers between two computers with 
wireless cards via ad hoc WiFi (or Wi-Fi Direct if necessary). No access
point, router, or other networking gear is required. Just select a file,
whether each computer is sending or receiving, and the operating system of the
other computer. Flying Carpet will do its best to restore your wireless
settings afterwards, but if there is an error, you may have to rejoin your 
wireless network manually. Thanks for using it and please provide feedback on
GitHub!`