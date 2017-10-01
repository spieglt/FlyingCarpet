package main

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"regexp"
	"strings"
	"time"
)

func (w *WindowsNetwork) startAdHoc(t *Transfer) bool {
	
	t.output(w.runCommand("netsh winsock reset"))
	w.stopAdHoc(t)
	t.output("SSID: " + t.SSID)
	t.output(w.runCommand("netsh wlan set hostednetwork mode=allow ssid=" + t.SSID + " key=" + t.Passphrase))
	_, err := exec.Command("netsh", "wlan", "start", "hostednetwork").CombinedOutput()
	if err != nil {
		w.teardown(t)
		t.output(fmt.Sprintf("Could not start hosted network. This computer's wireless card/driver may not support it. %s", err))
		return false
	}
	return true
}

func (w *WindowsNetwork) stopAdHoc(t *Transfer) {
	
	t.output(w.runCommand("netsh wlan stop hostednetwork"))
}

func (w *WindowsNetwork) joinAdHoc(t *Transfer) bool {
	
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
		w.teardown(t)
		t.output("Write error")
		return false
	}
	data := []byte(xmlDoc)
	if _, err = outFile.Write(data); err != nil {
		w.teardown(t)
		t.output("Write error")
		return false
	}
	defer os.Remove(tmpLoc)

	// add profile
	t.output(w.runCommand("netsh wlan add profile filename=" + tmpLoc + " user=current"))

	// join network
	timeout := JOIN_ADHOC_TIMEOUT
	t.output("")
	for t.SSID != w.getCurrentWifi() {
		if timeout <= 0 {
			t.output("Could not find the ad hoc network within the timeout period.")
			return false
		}
		cmdStr := "netsh wlan connect name=" + t.SSID
		cmdSlice := strings.Split(cmdStr, " ")
		_, cmdErr := exec.Command(cmdSlice[0], cmdSlice[1:]...).CombinedOutput()
		if cmdErr != nil {
			t.output(fmt.Sprintf("Failed to find the ad hoc network. Trying for %2d more seconds. %s", timeout, cmdErr))
		}
		timeout -= 5
		time.Sleep(time.Second * time.Duration(5))
	}
	// t.output("")
	return true
}

func (w *WindowsNetwork) findPeer(t *Transfer) (peerIP string) {
	
	ipPattern, _ := regexp.Compile("\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}")

	// clear arp cache
	w.runCommand("arp -d *")

	// get ad hoc ip
	var ifAddr string
	for !ipPattern.Match([]byte(ifAddr)) {
		// ifAddr = w.runCommand("$(ipconfig | Select-String -Pattern '(?<ipaddr>192\\.168\\.173\\..*)').Matches.Groups[1].Value.Trim()",
		// 	"Could not get ad hoc IP.")
		ifCmd := "$(ipconfig | Select-String -Pattern '(?<ipaddr>192\\.168\\.173\\..*)').Matches.Groups[1].Value.Trim()"
		ifBytes, err := exec.Command("powershell", "-c", ifCmd).CombinedOutput()
		if err != nil {
			t.output("Error getting ad hoc IP, retrying.")
		}
		ifAddr = strings.TrimSpace(string(ifBytes))
		// t.output("ad hoc IP:" + ifAddr)
		time.Sleep(time.Second * time.Duration(2))
	}
	t.output("Starting findPeer")
	// run arp for that ip
	for !ipPattern.Match([]byte(peerIP)) {

		// peerIP = w.runCommand("$(arp -a -N "+ifAddr+" | Select-String -Pattern '(?<ip>192\\.168\\.173\\.\\d{1,3})' | Select-String -NotMatch '(?<nm>("+
		// 	ifAddr+"|192.168.173.255)\\s)').Matches.Value",
		// 	"Could not get peer IP.")

		peerCmd := "$(arp -a -N " + ifAddr + " | Select-String -Pattern '(?<ip>192\\.168\\.173\\.\\d{1,3})' | Select-String -NotMatch '(?<nm>(" + ifAddr + "|192.168.173.255)\\s)').Matches.Value"
		peerBytes, err := exec.Command("powershell", "-c", peerCmd).CombinedOutput()
		if err != nil {
			t.output("Error getting ad hoc IP, retrying.")
		}
		peerIP = strings.TrimSpace(string(peerBytes))

		t.output(fmt.Sprintf("peer IP: %s", peerIP))
		time.Sleep(time.Second * time.Duration(2))
	}
	return
}

func (w WindowsNetwork) getCurrentWifi() (SSID string) {
	SSID = w.runCommand("$(netsh wlan show interfaces | Select-String -Pattern 'Profile *: (?<profile>.*)').Matches.Groups[1].Value.Trim()")
	return
}

func (w *WindowsNetwork) getWifiInterface() string {
	return ""
}

func (w WindowsNetwork) connectToPeer(t *Transfer) bool {
	
	if w.Mode == "receiving" {
		if !w.addFirewallRule(t) {
			return false
		}
		if !w.startAdHoc(t) {
			return false
		}
	} else if w.Mode == "sending" {
		if !w.checkForFile(t) {
			t.output(fmt.Sprintf("Could not find file to send: %s", t.Filepath))
			return false
		}
		if t.Peer == "windows" {
			if !w.joinAdHoc(t) {
				return false
			}
			t.RecipientIP = w.findPeer(t)
		} else if t.Peer == "mac" {
			if !w.addFirewallRule(t) {
				return false
			}
			if !w.startAdHoc(t) {
				return false
			}
			t.output("Ad hoc started, running findPeer")
			t.RecipientIP = w.findPeer(t)
		}
	}
	return true
}

func (w WindowsNetwork) resetWifi(t *Transfer) {
	
	if w.Mode == "receiving" || t.Peer == "mac" {
		w.deleteFirewallRule(t)
		w.stopAdHoc(t)
	} else {
		w.runCommand("netsh wlan delete profile name="+t.SSID)
		// rejoin previous wifi
		t.output(w.runCommand("netsh wlan connect name=" + w.PreviousSSID))
	}
}

func (w WindowsNetwork) addFirewallRule(t *Transfer) bool {
	
	execPath, err := os.Executable()
	if err != nil {
		t.output("Failed to get executable path.")
		return false
	}
	fwStr := "netsh advfirewall firewall add rule name=flyingcarpet dir=in action=allow program=" +
		execPath + " enable=yes profile=any localport=3290 protocol=tcp"
	fwSlice := strings.Split(fwStr, " ")
	_, err = exec.Command(fwSlice[0], fwSlice[1:]...).CombinedOutput()
	if err != nil {
		t.output("Could not create firewall rule. You must run as administrator to receive. (Press Win+X and then A to start an administrator command prompt.)")
		return false
	}
	t.output("Firewall rule created.")
	return true
}

func (w WindowsNetwork) deleteFirewallRule(t *Transfer) {
	fwStr := "netsh advfirewall firewall delete rule name=flyingcarpet"
	t.output(w.runCommand(fwStr))
}

func (w WindowsNetwork) checkForFile(t *Transfer) bool {
	_, err := os.Stat(t.Filepath)
	if err != nil {
		return false
	}
	return true
}

func (w *WindowsNetwork) runCommand(cmd string) (output string) {
	var cmdBytes []byte
	err := errors.New("")
	cmdSlice := strings.Split(cmd, " ")
	if len(cmdSlice) > 1 {
		cmdBytes, err = exec.Command(cmdSlice[0], cmdSlice[1:]...).CombinedOutput()
	} else {
		cmdBytes, err = exec.Command(cmd).CombinedOutput()
	}
	if err != nil {
		return err.Error()
	}
	return strings.TrimSpace(string(cmdBytes))
}

func (w WindowsNetwork) teardown(t *Transfer) {
	if w.Mode == "receiving" {
		os.Remove(t.Filepath)
	}
	w.resetWifi(t)
}
