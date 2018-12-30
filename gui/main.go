package main

import (
	"os"

	"github.com/therecipe/qt/widgets"
)

func main() {
	app := widgets.NewQApplication(len(os.Args), os.Args)
	gui := &Gui{}
	window := newWindow(gui)
	window.Show()
	app.Exec()
	return
}
