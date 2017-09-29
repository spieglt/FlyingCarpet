package main

import (
	"fmt"
	// "log"
	"os"
	"os/exec"
	"strings"
	"time"
)

func (m *MacNetwork) startAdHoc(t *Transfer) bool {
	tmpLoc := "/private/tmp/adhocnet"
	os.Remove(tmpLoc)

	data, err := Asset("static/adhocnet")
	if err != nil {
		m.teardown(t)
		OutputBox.AppendText("\nStatic file error")
		return false
	}
	outFile, err := os.OpenFile(tmpLoc, os.O_CREATE|os.O_RDWR, 0744)
	if err != nil {
		m.teardown(t)
		OutputBox.AppendText("\nError creating temp file")
		return false
	}
	if _, err = outFile.Write(data); err != nil {
		m.teardown(t)
		OutputBox.AppendText("\nWrite error")
		return false
	}
	defer os.Remove(tmpLoc)

	cmd := exec.Command(tmpLoc, t.SSID, t.Passphrase)
	output, err := cmd.CombinedOutput()
	if err != nil {
		fmt.Println(string(output))
		m.teardown(t)
		OutputBox.AppendText("\nError creating ad hoc network")
		return false
	}
	fmt.Printf("startAdHoc: %sSSID: %s\n", output, t.SSID)
	return true
}

func (m *MacNetwork) joinAdHoc(t *Transfer) bool {
	wifiInterface := m.getWifiInterface()
	OutputBox.AppendText("\nLooking for ad-hoc network...")
	timeout := JOIN_ADHOC_TIMEOUT
	joinAdHocStr := "networksetup -setairportnetwork " + wifiInterface + " " + t.SSID + " " + t.Passphrase
	joinAdHocBytes, err := exec.Command("sh", "-c", joinAdHocStr).CombinedOutput()
	OutputBox.AppendText("\n")
	for len(joinAdHocBytes) != 0 {
		if timeout <= 0 {
			OutputBox.AppendText("\nCould not find the ad hoc network within the timeout period.")
			return false
		}
		OutputBox.Replace(strings.LastIndex(OutputBox.GetValue(), "\n") + 1, OutputBox.GetLastPosition(), 
			fmt.Sprintf("\nFailed to join %s network. Trying for %2d more seconds.", t.SSID, timeout))
		timeout -= 5
		time.Sleep(time.Second * time.Duration(5))
		joinAdHocBytes, err = exec.Command("sh", "-c", joinAdHocStr).CombinedOutput()
		if err != nil {
			m.teardown(t)
			OutputBox.AppendText("\nError joining ad hoc network.")
			return false
		}
	}
	fmt.Printf("\n")
	return true
}

func (m MacNetwork) getCurrentWifi() (SSID string) {
	cmdStr := "/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport -I | awk '/ SSID/ {print substr($0, index($0, $2))}'"
	SSID = m.runCommand(cmdStr, "Could not get current SSID.")
	return
}

func (m *MacNetwork) getWifiInterface() string {
	getInterfaceString := "networksetup -listallhardwareports | awk '/Wi-Fi/{getline; print $2}'"
	return m.runCommand(getInterfaceString, "Could not get wifi interface.")
}

func (m *MacNetwork) findMac() (peerIP string, success bool) {
	timeout := FIND_MAC_TIMEOUT
	var currentIP string
	OutputBox.AppendText("\n")
	for currentIP == "" {
		currentIPString := "ipconfig getifaddr " + m.getWifiInterface()
		currentIPBytes, err := exec.Command("sh", "-c", currentIPString).CombinedOutput()
		if err != nil {
			OutputBox.Replace(strings.LastIndex(OutputBox.GetValue(), "\n") + 1, OutputBox.GetLastPosition(), 
				fmt.Sprintf("\nWaiting for self-assigned IP... %s", err))
			time.Sleep(time.Second * time.Duration(1))
			continue
		}
		currentIP = strings.TrimSpace(string(currentIPBytes))
	}
	fmt.Printf("\nSelf-assigned IP found: %s\n",currentIP)

	pingString := "ping -c 5 169.254.255.255 | " + // ping broadcast address
		"grep --line-buffered -oE '[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}' | " + // get all IPs
		"grep --line-buffered -vE '169.254.255.255' | " + // exclude broadcast address
		"grep -vE '" + currentIP + "'" // exclude current IP

	OutputBox.AppendText("\n")
	for peerIP == "" {
		if timeout <= 0 {
			OutputBox.AppendText("\nCould not find the peer computer within the timeout period.")
			return "", false
		}
		pingBytes, pingErr := exec.Command("sh", "-c", pingString).CombinedOutput()
		if pingErr != nil {
			OutputBox.Replace(strings.LastIndex(OutputBox.GetValue(), "\n") + 1, OutputBox.GetLastPosition(), 
				fmt.Sprintf("\nCould not find peer. Waiting %2d more seconds. %s",timeout,pingErr))
			timeout -= 2
			time.Sleep(time.Second * time.Duration(2))
			continue
		}
		peerIPs := string(pingBytes)
		peerIP = peerIPs[:strings.Index(peerIPs, "\n")]
	}
	fmt.Printf("\nPeer IP found: %s\n",peerIP)
	success = true
	return
}

func (m *MacNetwork) findWindows() (peerIP string) {
	return "192.168.173.1"
}

func (m MacNetwork) connectToPeer(t *Transfer) bool {
	if m.Mode == "sending" {
		if !m.checkForFile(t) {
			OutputBox.AppendText(fmt.Sprintf("\nCould not find file to send: %s",t.Filepath))
			return false
		}
		if !m.joinAdHoc(t) { return false }
		go m.stayOnAdHoc(t)
		if t.Peer == "mac" {
			var ok bool
			t.RecipientIP, ok = m.findMac()
			if !ok {
				return false
			}
		} else if t.Peer == "windows" {
			t.RecipientIP = m.findWindows()
		}
	} else if m.Mode == "receiving" {
		if t.Peer == "windows" {
			if !m.joinAdHoc(t) { return false }
			go m.stayOnAdHoc(t)
		} else if t.Peer == "mac" {
			if !m.startAdHoc(t) { return false }
		}
	}
	return true
}

func (m MacNetwork) resetWifi(t *Transfer) {
	wifiInterface := m.getWifiInterface()
	cmdString := "networksetup -setairportpower " + wifiInterface + " off && networksetup -setairportpower " + wifiInterface + " on"
	fmt.Println(m.runCommand(cmdString,"Could not reset WiFi adapter."))
}

func (m MacNetwork) stayOnAdHoc(t *Transfer) {
	for {
		select {
		case <- t.AdHocChan:
			OutputBox.AppendText("\nStopping ad hoc connection.")
			t.AdHocChan <- true
			return
		default:
			if m.getCurrentWifi() != t.SSID {
				m.joinAdHoc(t)
			}
			time.Sleep(time.Second * 1)
		}
	}
}

func (m MacNetwork) checkForFile(t *Transfer) bool {
	_,err := os.Stat(t.Filepath)
	if err != nil {
		return false
	}
	return true
}

func (m *MacNetwork) runCommand(cmd string, errDesc string) (output string) {
	cmdBytes, err := exec.Command("sh", "-c", cmd).CombinedOutput()
	if err != nil {
		fmt.Printf(errDesc+" Error: %s\n", err)
	}
	return strings.TrimSpace(string(cmdBytes))
}

func (m MacNetwork) teardown(t *Transfer) {
	if m.Mode == "receiving" {
		os.Remove(t.Filepath)
	}
	m.resetWifi(t)
}