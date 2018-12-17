package main

import (
	"github.com/therecipe/qt/widgets"
)

func showWindow() {
	window := widgets.NewQMainWindow(nil, 0)
	window.SetMinimumSize2(400, 600)
	window.SetWindowTitle("Flying Carpet")

	widget := widgets.NewQWidget(nil, 0)
	widget.SetLayout(widgets.NewQVBoxLayout())
	window.SetCentralWidget(widget)

	radioWidget := widgets.NewQWidget(nil, 0)
	radioWidget.SetLayout(widgets.NewQHBoxLayout())

	peerWidget := widgets.NewQWidget(nil, 0)
	peerWidget.SetWindowTitle("this is that title")
	peerLayout := widgets.NewQVBoxLayout2(peerWidget)
	peerLayout.AddWidget(widgets.NewQRadioButton2("Linux", nil), 0, 0)
	peerLayout.AddWidget(widgets.NewQRadioButton2("Mac", nil), 0, 0)
	peerLayout.AddWidget(widgets.NewQRadioButton2("Windows", nil), 0, 0)

	modeWidget := widgets.NewQWidget(nil, 0)
	modeWidget.SetLayout(widgets.NewQVBoxLayout())

	radioWidget.Layout().AddWidget(peerWidget)
	radioWidget.Layout().AddWidget(modeWidget)
	widget.Layout().AddWidget(radioWidget)

	// bottom half
	bottomWidget := widgets.NewQWidget(nil, 0)
	bottomWidget.SetLayout(widgets.NewQVBoxLayout())
	fileWidget := widgets.NewQWidget(nil, 0)
	fileWidget.SetLayout(widgets.NewQHBoxLayout())
	fileBox := widgets.NewQLineEdit(nil)
	sendButton := widgets.NewQPushButton2("Select file(s)...", nil)
	receiveButton := widgets.NewQPushButton2("Select folder...", nil)
	receiveButton.Hide()
	fileWidget.Layout().AddWidget(sendButton)
	fileWidget.Layout().AddWidget(receiveButton)
	fileWidget.Layout().AddWidget(fileBox)
	bottomWidget.Layout().AddWidget(fileWidget)
	widget.Layout().AddWidget(bottomWidget)

	// input := widgets.NewQLineEdit(nil)
	// input.SetPlaceholderText("Write something ...")
	// widget.Layout().AddWidget(input)

	// create a button
	// connect the clicked signal
	// and add it to the central widgets layout
	// button := widgets.NewQPushButton2("and click me!", nil)
	// button.ConnectClicked(func(bool) {
	// 	widgets.QMessageBox_Information(nil, "OK", input.Text(), widgets.QMessageBox__Ok, widgets.QMessageBox__Ok)
	// })
	// widget.Layout().AddWidget(button)

	window.Show()
}

func (t *Transfer) output(msg string) {

	//for testing
	// file, err := os.OpenFile("err.txt", os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
	// if err != nil {
	// 	panic(err)
	// }
	// defer file.Close()
	// file.WriteString(msg)
	// file.WriteString("\r\n")
}
