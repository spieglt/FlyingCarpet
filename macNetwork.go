package main

import (
	"fmt"
	"log"
	"os"
	"os/exec"
	"strings"
	"time"
)

func (m *MacNetwork) startAdHoc(t *Transfer) {
	tmpLoc := "/private/tmp/adhocnet"
	os.Remove(tmpLoc)

	data, err := Asset("static/adhocnet")
	if err != nil {
		m.teardown(t)
		log.Fatal("Static file error")
	}
	outFile, err := os.OpenFile(tmpLoc, os.O_CREATE|os.O_RDWR, 0744)
	if err != nil {
		m.teardown(t)
		log.Fatal("Error creating temp file")
	}
	if _, err = outFile.Write(data); err != nil {
		m.teardown(t)
		log.Fatal("Write error")
	}
	defer os.Remove(tmpLoc)

	cmd := exec.Command(tmpLoc, t.SSID, t.Passphrase)
	output, err := cmd.CombinedOutput()
	if err != nil {
		fmt.Println(string(output))
		m.teardown(t)
		log.Fatal("Error creating ad hoc network")
	}
	fmt.Printf("startAdHoc: %sSSID: %s\n", output, t.SSID)

}

func (m *MacNetwork) joinAdHoc(t *Transfer) {
	wifiInterface := m.getWifiInterface()
	fmt.Println("Looking for ad-hoc network...")
	timeout := JOIN_ADHOC_TIMEOUT
	joinAdHocStr := "networksetup -setairportnetwork " + wifiInterface + " " + t.SSID + " " + t.Passphrase
	joinAdHocBytes, err := exec.Command("sh", "-c", joinAdHocStr).CombinedOutput()
	for len(joinAdHocBytes) != 0 {
		if timeout <= 0 {
			log.Fatal("Could not find the ad hoc network within the timeout period. Exiting.")
		}
		fmt.Printf("\rFailed to join %s network. Trying for %2d more seconds.", t.SSID, timeout)
		timeout -= 5
		time.Sleep(time.Second * time.Duration(5))
		joinAdHocBytes, err = exec.Command("sh", "-c", joinAdHocStr).CombinedOutput()
		if err != nil {
			m.teardown(t)
			log.Fatal("Error joining ad hoc network.")
		}
	}
	fmt.Printf("\n")
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

func (m *MacNetwork) findMac() (peerIP string) {
	timeout := FIND_MAC_TIMEOUT
	var currentIP string
	for currentIP == "" {
		currentIPString := "ipconfig getifaddr " + m.getWifiInterface()
		currentIPBytes, err := exec.Command("sh", "-c", currentIPString).CombinedOutput()
		if err != nil {
			fmt.Printf("\rWaiting for self-assigned IP... %s", err)
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

	for peerIP == "" {
		if timeout <= 0 {
			log.Fatal("Could not find the peer computer within the timeout period. Exiting.")
		}
		pingBytes, pingErr := exec.Command("sh", "-c", pingString).CombinedOutput()
		if pingErr != nil {
			fmt.Printf("\rCould not find peer. Waiting %2d more seconds. %s",timeout,pingErr)
			timeout -= 2
			time.Sleep(time.Second * time.Duration(2))
			continue
		}
		peerIPs := string(pingBytes)
		peerIP = peerIPs[:strings.Index(peerIPs, "\n")]
	}
	fmt.Printf("\nPeer IP found: %s\n",peerIP)
	return
}

func (m *MacNetwork) findWindows() (peerIP string) {
	return "192.168.173.1"
}

func (m MacNetwork) connectToPeer(t *Transfer) {
	if m.Mode == "sending" {
		if !m.checkForFile(t) {
			log.Fatal("Could not find file to send: ",t.Filepath)
		}
		m.joinAdHoc(t)
		go m.stayOnAdHoc(t)
		if t.Peer == "mac" {
			t.RecipientIP = m.findMac()
		} else if t.Peer == "windows" {
			t.RecipientIP = m.findWindows()
		}
	} else if m.Mode == "receiving" {
		if t.Peer == "windows" {
			m.joinAdHoc(t)
			go m.stayOnAdHoc(t)
		} else if t.Peer == "mac" {
			m.startAdHoc(t)
		}
	}
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