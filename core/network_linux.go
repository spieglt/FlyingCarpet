package core

import (
	"errors"
	"fmt"
	"os/exec"
	"strings"
	"time"
)

func connectToPeer(t *Transfer, ui UI) (err error) {
	if t.Mode == "sending" {
		if t.Peer == "mac" {
			if err = startAdHoc(t, ui); err != nil {
				return
			}
			t.RecipientIP, err = findMac(t, ui)
			if err != nil {
				return
			}
		} else if t.Peer == "windows" {
			if err = joinAdHoc(t, ui); err != nil {
				return
			}
			t.RecipientIP = findWindows(t)
		} else if t.Peer == "linux" {
			if err = joinAdHoc(t, ui); err != nil {
				return
			}
			t.RecipientIP = findLinux(t)
		}
	} else if t.Mode == "receiving" {
		if t.Peer == "windows" {
			if err = joinAdHoc(t, ui); err != nil {
				return
			}
		} else if t.Peer == "mac" {
			if err = startAdHoc(t, ui); err != nil {
				return
			}
		} else if t.Peer == "linux" {
			if err = startAdHoc(t, ui); err != nil {
				return
			}
		}
	}
	return
}

// TODO: fix this function, add error handling.
func startAdHoc(t *Transfer, ui UI) (err error) {
	// or just:
	// nmcli dev wifi hotspot ssid t.SSID band bg channel 11 password t.Password + t.Password
	// ??
	commands := []string{"nmcli con add type wifi ifname " + getWifiInterface() + " con-name " + t.SSID + " autoconnect yes ssid " + t.SSID,
		"nmcli con modify " + t.SSID + " 802-11-wireless.mode ap 802-11-wireless.band bg ipv4.method shared",
		"nmcli con modify " + t.SSID + " wifi-sec.key-mgmt wpa-psk",
		"nmcli con modify " + t.SSID + " wifi-sec.psk \"" + t.Password + t.Password + "\"",
		"nmcli con up " + t.SSID}
	for _, cmd := range commands {
		out := runCommand(cmd)
		if out != "" {
			ui.Output(out)
		}
	}
	return
}

// TODO: fix this function, add error handling.
func joinAdHoc(t *Transfer, ui UI) (err error) {
	ui.Output("Looking for ad-hoc network " + t.SSID)
	var outBytes []byte
	commands := []string{"nmcli con add type wifi ifname " + getWifiInterface() + " con-name \"" + t.SSID + "\" autoconnect yes ssid \"" + t.SSID + "\"",
		"nmcli con modify \"" + t.SSID + "\" wifi-sec.key-mgmt wpa-psk",
		"nmcli con modify \"" + t.SSID + "\" wifi-sec.psk \"" + t.Password + t.Password + "\"",
		"nmcli con up \"" + t.SSID + "\""}
	for i, cmd := range commands {
		outBytes, err = exec.Command("sh", "-c", cmd).CombinedOutput()
		// ui.Output(fmt.Sprintf("outBytes %d: %s", i, string(outBytes)))
		if err != nil {
			ui.Output(fmt.Sprintf("Error %d: %s", i, err.Error()))
		}
	}
	for string(outBytes)[:5] == "Error" {
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting joinAdHoc, transfer was canceled.")
		default:
			time.Sleep(time.Second * time.Duration(5))
			outBytes, err = exec.Command("sh", "-c", "nmcli con up \""+t.SSID+"\"").CombinedOutput()
			ui.Output(string(outBytes))
			if err != nil {
				ui.Output(fmt.Sprintf("Error joining ad hoc network: %s", err))
			}
		}
	}
	ui.Output(string(outBytes))
	return
}

func resetWifi(t *Transfer, ui UI) {
	command := "nmcli con down \"" + t.SSID + "\""
	ui.Output(runCommand(command))
	command = "nmcli con delete \"" + t.SSID + "\""
	ui.Output(runCommand(command))
	command = "nmcli con up \"" + t.PreviousSSID + "\""
	ui.Output(runCommand(command))
	return
}

func getCurrentWifi(ui UI) (ssid string) {
	command := "nmcli -f active,ssid dev wifi | awk '/^yes/{print $2}"
	ssid = runCommand(command)
	return
}

func getCurrentUUID() (uuid string) {
	command := "nmcli -f active,uuid con | awk '/^yes/{print $2}'"
	uuid = runCommand(command)
	return
}

func getWifiInterface() (iface string) {
	command := "ifconfig | awk '/^wl/{print $1}'"
	iface = runCommand(command)
	return
}

func getIPAddress(t *Transfer) (ip string) {
	command := "ifconfig wlp2s0 | awk '{print $2}' | grep -oP 'addr:\\K.*'"
	ip = runCommand(command)
	return
}

func findMac(t *Transfer, ui UI) (peerIP string, err error) {
	currentIP := getIPAddress(t)
	pingString := "ping -b -c 5 $(ifconfig | awk '/Bcast/ {print substr($3,7)}') 2>&1 | " + // ping broadcast address, include stderr
		"grep --line-buffered -oE '[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}' | " + // get all IPs
		"grep --line-buffered -vE $(ifconfig | awk '/Bcast/ {print substr($3,7)}') | " + // exclude broadcast address
		"grep -vE '" + currentIP + "'" // exclude current IP

	ui.Output("Looking for peer IP.")
	for peerIP == "" {
		pingBytes, pingErr := exec.Command("sh", "-c", pingString).CombinedOutput()
		if pingErr != nil {
			time.Sleep(time.Second * time.Duration(2))
			continue
		}
		peerIPs := string(pingBytes)
		peerIP = peerIPs[:strings.Index(peerIPs, "\n")]
	}
	ui.Output(fmt.Sprintf("Peer IP found: %s", peerIP))
	return
}

func findWindows(t *Transfer) string {
	currentIP := getIPAddress(t)
	if strings.Contains(currentIP, "192.168.137") {
		return "192.168.137.1"
	}
	return "192.168.173.1"
}

func findLinux(t *Transfer) string {
	return "10.42.0.1"
}

func runCommand(cmd string) (output string) {
	cmdBytes, err := exec.Command("sh", "-c", cmd).CombinedOutput()
	if err != nil {
		return err.Error()
	}
	return strings.TrimSpace(string(cmdBytes))
}
