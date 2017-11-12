package main

import "github.com/dontpanic92/wxGo/wx"

func addAboutToOSXMenu(menuBar wx.MenuBar) {
	menu := wx.NewMenu()
	menu = menuBar.OSXGetAppleMenu()
	menu.Append(wx.ID_ABOUT)
}
