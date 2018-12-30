package main

import (
	"github.com/spieglt/flyingcarpet/core"
)

func main() {
	cli := &Cli{}
	t := getInput(cli)
	core.StartTransfer(t, cli)
}
