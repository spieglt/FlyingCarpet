package main

import (
	"fmt"
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
		t.output("Static file error")
		return false
	}
	outFile, err := os.OpenFile(tmpLoc, os.O_CREATE|os.O_RDWR, 0744)
	if err != nil {
		m.teardown(t)
		t.output("Error creating temp file")
		return false
	}
	if _, err = outFile.Write(data); err != nil {
		m.teardown(t)
		t.output("Write error")
		return false
	}
	defer os.Remove(tmpLoc)

	cmd := exec.Command(tmpLoc, t.SSID, t.Passphrase)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.output(string(output))
		m.teardown(t)
		t.output("Error creating ad hoc network")
		return false
	}
	t.output(fmt.Sprintf("startAdHoc: %sSSID: %s\n", output, t.SSID))
	return true
}

func (m *MacNetwork) joinAdHoc(t *Transfer) bool {
	
	wifiInterface := m.getWifiInterface()
	t.output("Looking for ad-hoc network " + t.SSID + "...")
	timeout := JOIN_ADHOC_TIMEOUT
	joinAdHocStr := "networksetup -setairportnetwork " + wifiInterface + " " + t.SSID + " " + t.Passphrase
	joinAdHocBytes, err := exec.Command("sh", "-c", joinAdHocStr).CombinedOutput()
	for len(joinAdHocBytes) != 0 {
		if timeout <= 0 {
			t.output("Could not find the ad hoc network within the timeout period.")
			return false
		}
		t.output(fmt.Sprintf("Failed to join the ad hoc network. Trying for %2d more seconds.", timeout))
		timeout -= 5
		time.Sleep(time.Second * time.Duration(5))
		joinAdHocBytes, err = exec.Command("sh", "-c", joinAdHocStr).CombinedOutput()
		if err != nil {
			m.teardown(t)
			t.output("Error joining ad hoc network.")
			return false
		}
	}
	return true
}

func (m MacNetwork) getCurrentWifi() (SSID string) {
	cmdStr := "/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport -I | awk '/ SSID/ {print substr($0, index($0, $2))}'"
	SSID = m.runCommand(cmdStr)
	return
}

func (m *MacNetwork) getWifiInterface() string {
	getInterfaceString := "networksetup -listallhardwareports | awk '/Wi-Fi/{getline; print $2}'"
	return m.runCommand(getInterfaceString)
}

func (m *MacNetwork) findMac(t *Transfer) (peerIP string, success bool) {
	timeout := FIND_MAC_TIMEOUT
	var currentIP string
	for currentIP == "" {
		currentIPString := "ipconfig getifaddr " + m.getWifiInterface()
		currentIPBytes, err := exec.Command("sh", "-c", currentIPString).CombinedOutput()
		if err != nil {
			t.output(fmt.Sprintf("Waiting for self-assigned IP... %s", err))
			time.Sleep(time.Second * time.Duration(1))
			continue
		}
		currentIP = strings.TrimSpace(string(currentIPBytes))
	}

	t.output(fmt.Sprintf("Self-assigned IP found: %s", currentIP))

	pingString := "ping -c 5 169.254.255.255 | " + // ping broadcast address
		"grep --line-buffered -oE '[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}' | " + // get all IPs
		"grep --line-buffered -vE '169.254.255.255' | " + // exclude broadcast address
		"grep -vE '" + currentIP + "'" // exclude current IP

	for peerIP == "" {
		if timeout <= 0 {
			t.output("Could not find the peer computer within the timeout period.")
			return "", false
		}
		pingBytes, pingErr := exec.Command("sh", "-c", pingString).CombinedOutput()
		if pingErr != nil {
			t.output(fmt.Sprintf("Could not find peer. Waiting %2d more seconds. %s", timeout, pingErr))
			timeout -= 2
			time.Sleep(time.Second * time.Duration(2))
			continue
		}
		peerIPs := string(pingBytes)
		peerIP = peerIPs[:strings.Index(peerIPs, "\n")]
	}
	t.output(fmt.Sprintf("Peer IP found: %s", peerIP))
	success = true
	return
}

func (m *MacNetwork) findWindows() (peerIP string) {
	return "192.168.173.1"
}

func (m MacNetwork) connectToPeer(t *Transfer) bool {
	
	if m.Mode == "sending" {
		if !m.checkForFile(t) {
			t.output(fmt.Sprintf("Could not find file to send: %s", t.Filepath))
			return false
		}
		if !m.joinAdHoc(t) {
			return false
		}
		go m.stayOnAdHoc(t)
		if t.Peer == "mac" {
			var ok bool
			t.RecipientIP, ok = m.findMac(t)
			if !ok {
				return false
			}
		} else if t.Peer == "windows" {
			t.RecipientIP = m.findWindows()
		}
	} else if m.Mode == "receiving" {
		if t.Peer == "windows" {
			if !m.joinAdHoc(t) {
				return false
			}
			go m.stayOnAdHoc(t)
		} else if t.Peer == "mac" {
			if !m.startAdHoc(t) {
				return false
			}
		}
	}
	return true
}

func (m MacNetwork) resetWifi(t *Transfer) {
	
	wifiInterface := m.getWifiInterface()
	cmdString := "networksetup -setairportpower " + wifiInterface + " off && networksetup -setairportpower " + wifiInterface + " on"
	t.output(m.runCommand(cmdString))
}

func (m MacNetwork) stayOnAdHoc(t *Transfer) {
	
	for {
		select {
		case <-t.AdHocChan:
			t.output("Stopping ad hoc connection.")
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
	_, err := os.Stat(t.Filepath)
	if err != nil {
		return false
	}
	return true
}

func (m *MacNetwork) runCommand(cmd string) (output string) {
	cmdBytes, err := exec.Command("sh", "-c", cmd).CombinedOutput()
	if err != nil {
		return err.Error()
	}
	return strings.TrimSpace(string(cmdBytes))
}

func (m MacNetwork) teardown(t *Transfer) {
	if m.Mode == "receiving" {
		os.Remove(t.Filepath)
	}
	m.resetWifi(t)
}
