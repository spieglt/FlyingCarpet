package main

import (
	"context"
	"os"

	"github.com/spieglt/flyingcarpet/core"
	"github.com/therecipe/qt/widgets"
)

func main() {
	app := widgets.NewQApplication(len(os.Args), os.Args)
	gui := &Gui{}
	t := newTransfer()
	window := newWindow(t, gui)
	window.Show()
	app.Exec()
	return
}

func newTransfer() (t *core.Transfer) {
	t.WfdSendChan, t.WfdRecvChan = make(chan string), make(chan string)
	t.Ctx, t.CancelCtx = context.WithCancel(context.Background())
	t.Port = 3290
	return
}
