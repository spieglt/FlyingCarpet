package main

import (
	"github.com/spieglt/flyingcarpet/core"
)

func main() {
	cli := &Cli{}
	t := getInput(cli)
	cli.Output("Welcome to Flying Carpet!")
	core.StartTransfer(t, cli)
}
