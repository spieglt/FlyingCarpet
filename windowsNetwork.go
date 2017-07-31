package main

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
	"log"
	"regexp"
	"time"
)

func (w *WindowsNetwork) startAdHoc(t *Transfer) {
	// fmt.Println(w.runCommand("netsh winsock reset", "Could not reset network adapter."))
	fmt.Println("SSID:", t.SSID)
	fmt.Println(w.runCommand("netsh wlan set hostednetwork mode=allow ssid="+t.SSID+" key="+t.Passphrase,
		"Could not set hosted network settings."))
	fmt.Println(w.runCommand("netsh wlan start hostednetwork", "Could not start hosted network."))
}

func (w *WindowsNetwork) stopAdHoc() {
	fmt.Println(w.runCommand("netsh wlan stop hostednetwork", "Failed to stop hosted network."))
}

func (w *WindowsNetwork) joinAdHoc(t *Transfer) {	
	tmpLoc := ".\\adhoc.xml"

	// make doc
	xmlDoc := "<?xml version=\"1.0\"?>\r\n" +
		"<WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\r\n" +
		"	<name>" + t.SSID + "</name>\r\n" +
		"	<SSIDConfig>\r\n" +
		"		<SSID>\r\n" +
		"			<name>" + t.SSID + "</name>\r\n" +
		"		</SSID>\r\n" +
		"	</SSIDConfig>\r\n" +
		"	<connectionType>ESS</connectionType>\r\n" +
		"	<connectionMode>auto</connectionMode>\r\n" +
		"	<MSM>\r\n" +
		"		<security>\r\n" +
		"			<authEncryption>\r\n" +
		"				<authentication>WPA2PSK</authentication>\r\n" +
		"				<encryption>AES</encryption>\r\n" +
		"				<useOneX>false</useOneX>\r\n" +
		"			</authEncryption>\r\n" +
		"			<sharedKey>\r\n" +
		"				<keyType>passPhrase</keyType>\r\n" +
		"				<protected>false</protected>\r\n" +
		"				<keyMaterial>" + t.Passphrase + "</keyMaterial>\r\n" +
		"			</sharedKey>\r\n" +
		"		</security>\r\n" +
		"	</MSM>\r\n" +
		"	<MacRandomization xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v3\">\r\n" +
		"		<enableRandomization>false</enableRandomization>\r\n" +
		"	</MacRandomization>\r\n" +
		"</WLANProfile>"
	// delete file if there
	os.Remove(tmpLoc)

	// write file
	outFile, err := os.OpenFile(tmpLoc, os.O_CREATE|os.O_RDWR, 0744)
	if err != nil {
		panic(err)
	}
	data := []byte(xmlDoc)
	if _, err = outFile.Write(data); err != nil {
		panic(err)
	}
	defer os.Remove(tmpLoc)

	// reset network adapter
	// fmt.Println(w.runCommand("netsh winsock reset", "Could not reset network adapter."))

	// add profile
	fmt.Println(w.runCommand("netsh wlan add profile filename="+tmpLoc+" user=current", "Could not add wireless profile."))

	// join network
	timeout := JOIN_ADHOC_TIMEOUT
	for t.SSID != w.getCurrentWifi() {
		if timeout <= 0 {
			log.Fatal("Could not find the ad hoc network within the timeout period. Exiting.")
		}
		cmdStr := "netsh wlan connect name=" + t.SSID
		_,cmdErr := exec.Command("powershell", "-c", cmdStr).CombinedOutput()
		if cmdErr != nil {
			fmt.Printf("\rFailed to find the ad hoc network. Trying for %3d more seconds. %s",timeout,cmdErr)
		}
		timeout -= 5
		time.Sleep(time.Second * time.Duration(5))
	}
	fmt.Println("\n")
}

func (w *WindowsNetwork) findPeer() (peerIP string) {
	ipPattern, _ := regexp.Compile("\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}")

	// clear arp cache
	w.runCommand("arp -d *", "Could not clear arp cache.")

	// get ad hoc ip
	var ifAddr string
	for !ipPattern.Match([]byte(ifAddr)) {
		ifAddr = w.runCommand("$(ipconfig | Select-String -Pattern '(?<ipaddr>192\\.168\\.173\\..*)').Matches.Groups[1].Value.Trim()",
			"Could not get ad hoc IP.")
		fmt.Println("ad hoc IP:", ifAddr)
		time.Sleep(time.Second * time.Duration(2))
	}

	// run arp for that ip
	for !ipPattern.Match([]byte(peerIP)) {
		peerIP = w.runCommand("$(arp -a -N "+ifAddr+" | Select-String -Pattern '(?<ip>192\\.168\\.173\\.\\d{1,3})' | Select-String -NotMatch '(?<nm>("+
			ifAddr+"|192.168.173.255)\\s)').Matches.Value",
			"Could not get peer IP.")
		fmt.Println("peer IP:", peerIP)
		time.Sleep(time.Second * time.Duration(2))
	}
	return
}

func (w WindowsNetwork) getCurrentWifi() (SSID string) {
	SSID = w.runCommand("$(netsh wlan show interfaces | Select-String -Pattern 'Profile *: (?<profile>.*)').Matches.Groups[1].Value.Trim()",
		"Could not get current SSID.")
	return
}

func (w *WindowsNetwork) getWifiInterface() string {
	return ""
}

func (w WindowsNetwork) connectToPeer(t *Transfer) {
	if w.Mode == "receiving" {
		w.startAdHoc(t)
	} else if w.Mode == "sending" {
		if t.Peer == "windows" {
			w.joinAdHoc(t)
			t.RecipientIP = w.findPeer()
		} else if t.Peer == "mac" {
			w.startAdHoc(t)
			t.RecipientIP = w.findPeer()
		}
	}
}

func (w WindowsNetwork) resetWifi(t *Transfer) {
	if w.Mode == "receiving" || t.Peer == "mac" {
		// stop ad hoc
		w.stopAdHoc()
	} else {
		w.runCommand("netsh wlan delete profile name=" + t.SSID, "Could not delete ad hoc profile.")
		// rejoin previous wifi
		fmt.Println(w.runCommand("netsh wlan connect name=" + w.PreviousSSID, "Could not join ad hoc network."))
	}
}

func (w *WindowsNetwork) runCommand(cmd string, errDesc string) (output string) {
	cmdBytes, err := exec.Command("powershell", "-Command", cmd).CombinedOutput()
	if err != nil {
		fmt.Printf(errDesc+" Error: %s\n", err)
	}
	return strings.TrimSpace(string(cmdBytes))
}

func (w *WindowsNetwork) teardown(t *Transfer) {
	if w.Mode == "receiving" {
		os.Remove(t.Filepath)
	}
	w.resetWifi(t)
}