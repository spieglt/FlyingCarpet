package main

import (
	rice "github.com/GeertJohan/go.rice"
)

func getBox() (*rice.Box, error) {
	return rice.FindBox(".\\flyingcarpet\\deploy\\windows")
}
