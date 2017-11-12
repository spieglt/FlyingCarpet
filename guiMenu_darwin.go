package main

import "github.com/dontpanic92/wxGo/wx"

func addAboutToOSXMenu() {
	menu := wx.NewMenu()
	menu = wx.OSXGetAppleMenu()
	menu.Append(wx.ID_ABOUT)
}