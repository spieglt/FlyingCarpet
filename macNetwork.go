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
	tmpLoc := "/tmp/adhocnet"
	os.Remove(tmpLoc)

	data, err := Asset("static/adhocnet")
	if err != nil {
		panic(err)
	}
	outFile, err := os.OpenFile(tmpLoc, os.O_CREATE|os.O_RDWR, 0744)
	if err != nil {
		panic(err)
	}
	if _, err = outFile.Write(data); err != nil {
		panic(err)
	}
	defer os.Remove(tmpLoc)

	// to clear arp cache, open tcp sockets?
	m.resetWifi(t)

	cmd := exec.Command(tmpLoc, t.SSID, t.Passphrase)
	output, err := cmd.CombinedOutput()
	if err != nil {
		fmt.Println(string(output))
		panic(err)
	}
	fmt.Printf("startAdHoc: %s\n", output)

}

func (m *MacNetwork) joinAdHoc(t *Transfer) {
	// to clear arp cache, open tcp sockets?
	m.resetWifi(t)

	wifiInterface := m.getWifiInterface()
	fmt.Println("Looking for ad-hoc network...")
	timeout := JOIN_ADHOC_TIMEOUT
	joinAdHocStr := "networksetup -setairportnetwork " + wifiInterface + " " + t.SSID + " " + t.Passphrase
	joinAdHocBytes, err := exec.Command("sh", "-c", joinAdHocStr).CombinedOutput()
	for len(joinAdHocBytes) != 0 {
		if timeout <= 0 {
			log.Fatal("Could not find the ad hoc network within the timeout period. Exiting.")
		}
		fmt.Printf("\rFailed to join %s network. Trying for %3d more seconds.", t.SSID, timeout)
		timeout -= 5
		time.Sleep(time.Second * time.Duration(5))
		joinAdHocBytes, err = exec.Command("sh", "-c", joinAdHocStr).CombinedOutput()
		if err != nil {
			panic(err)
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
			fmt.Printf("\rCould not find peer. Waiting %3d more seconds. %s",timeout,pingErr)
			timeout -= 5
			time.Sleep(time.Second * time.Duration(5))
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
		m.joinAdHoc(t)
		if t.Peer == "mac" {
			t.RecipientIP = m.findMac()
		} else if t.Peer == "windows" {
			t.RecipientIP = m.findWindows()
		}
	} else if m.Mode == "receiving" {
		if t.Peer == "windows" {
			m.joinAdHoc(t)
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

func (m *MacNetwork) runCommand(cmd string, errDesc string) (output string) {
	cmdBytes, err := exec.Command("sh", "-c", cmd).CombinedOutput()
	if err != nil {
		fmt.Printf(errDesc+" Error: %s\n", err)
	}
	return strings.TrimSpace(string(cmdBytes))
}

func (m *MacNetwork) teardown(t *Transfer) {
	if m.Mode == "receiving" {
		os.Remove(t.Filepath)
	}
	m.resetWifi(t)
}