package main

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"regexp"
	"strings"
	"syscall"
	"time"
)

func (n *Network) startAdHoc(t *Transfer) bool {

	n.runCommand("netsh winsock reset")
	n.runCommand("netsh wlan stop hostednetwork")
	t.output("SSID: " + t.SSID)
	n.runCommand("netsh wlan set hostednetwork mode=allow ssid=" + t.SSID + " key=" + t.Passphrase + t.Passphrase)
	cmd := exec.Command("netsh", "wlan", "start", "hostednetwork")
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	_, err := cmd.CombinedOutput()
	// TODO: replace with "echo %errorlevel%" == "1"
	if err.Error() == "exit status 1" {
		t.output("Could not start hosted network, trying Wi-Fi Direct.")
		n.AdHocCapable = false

		startChan := make(chan bool)
		go n.startLegacyAP(t, startChan)
		if ok := <-startChan; !ok {
			return false
		}
		return true
	} else if err == nil {
		n.AdHocCapable = true
		return true
	} else {
		t.output(fmt.Sprintf("Could not start hosted network."))
		n.teardown(t)
		return false
	}
}

func (n *Network) stopAdHoc(t *Transfer) {
	if n.AdHocCapable {
		t.output(n.runCommand("netsh wlan stop hostednetwork"))
	} else {
		t.output("Stopping Wi-Fi Direct.")
		// TODO: blocking operation, check wifiDirect function is running.
		n.WifiDirectChan <- "quit"
		reply := <-n.WifiDirectChan
		t.output(reply)
		close(n.WifiDirectChan)
	}
}

func (n *Network) joinAdHoc(t *Transfer) bool {
	cmd := exec.Command("cmd", "/C", "echo %TEMP%")
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	cmdBytes, err := cmd.CombinedOutput()
	if err != nil {
		t.output("Error getting temp location.")
		return false
	}
	tmpLoc := strings.TrimSpace(string(cmdBytes)) + "\\adhoc.xml"

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
		"				<keyMaterial>" + t.Passphrase + t.Passphrase + "</keyMaterial>\r\n" +
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
		n.teardown(t)
		t.output("Write error")
		return false
	}
	data := []byte(xmlDoc)
	if _, err = outFile.Write(data); err != nil {
		n.teardown(t)
		t.output("Write error")
		return false
	}
	defer os.Remove(tmpLoc)

	// add profile
	t.output(n.runCommand("netsh wlan add profile filename=" + tmpLoc + " user=current"))

	// join network
	t.output("Looking for ad-hoc network " + t.SSID + "...")
	timeout := JOIN_ADHOC_TIMEOUT
	for t.SSID != n.getCurrentWifi(t) {
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
	return true
}

func (n *Network) findPeer(t *Transfer) (peerIP string) {

	ipPattern, _ := regexp.Compile("\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}")

	// clear arp cache
	n.runCommand("arp -d *")

	// get ad hoc ip
	var ifAddr string
	for !ipPattern.Match([]byte(ifAddr)) {
		ifString := "$(ipconfig | Select-String -Pattern '(?<ipaddr>192\\.168\\.\\d{1,3}\\..*)').Matches.Groups[1].Value.Trim()"
		ifCmd := exec.Command("powershell", "-c", ifString)
		ifCmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
		ifBytes, err := ifCmd.CombinedOutput()
		if err != nil {
			t.output("Error getting ad hoc IP, retrying.")
		}
		ifAddr = strings.TrimSpace(string(ifBytes))
		time.Sleep(time.Second * time.Duration(2))
	}

	// necessary for wifi direct ip addresses
	var thirdOctet string
	if strings.Contains(ifAddr, "137") {
		thirdOctet = "137"
	} else {
		thirdOctet = "173"
	}

	// run arp for that ip
	for !ipPattern.Match([]byte(peerIP)) {
		peerString := "$(arp -a -N " + ifAddr + " | Select-String -Pattern '(?<ip>192\\.168\\." + thirdOctet + "\\.\\d{1,3})' | Select-String -NotMatch '(?<nm>(" + ifAddr + "|192.168." + thirdOctet + ".255)\\s)').Matches.Value"
		peerCmd := exec.Command("powershell", "-c", peerString)
		peerCmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
		peerBytes, err := peerCmd.CombinedOutput()
		if err != nil {
			t.output("Error getting ad hoc IP, retrying.")
		}
		peerIP = strings.TrimSpace(string(peerBytes))
		time.Sleep(time.Second * time.Duration(2))
	}
	t.output(fmt.Sprintf("peer IP: %s", peerIP))
	return
}

func (n *Network) getCurrentWifi(t *Transfer) (SSID string) {
	cmdStr := "$(netsh wlan show interfaces | Select-String -Pattern 'Profile *: (?<profile>.*)').Matches.Groups[1].Value.Trim()"
	cmd := exec.Command("powershell", "-c", cmdStr)
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	cmdBytes, err := cmd.CombinedOutput()
	if err != nil {
		t.output("Error getting current SSID.")
	}
	SSID = strings.TrimSpace(string(cmdBytes))
	return
}

func (n *Network) getWifiInterface() string {
	return ""
}

func (n *Network) connectToPeer(t *Transfer) bool {

	if n.Mode == "receiving" {
		if !n.addFirewallRule(t) {
			return false
		}
		if !n.startAdHoc(t) {
			return false
		}
	} else if n.Mode == "sending" {
		if !n.checkForFile(t) {
			t.output(fmt.Sprintf("Could not find file to send: %s", t.Filepath))
			return false
		}
		if t.Peer == "windows" {
			if !n.joinAdHoc(t) {
				return false
			}
			t.RecipientIP = n.findPeer(t)
		} else if t.Peer == "mac" {
			if !n.addFirewallRule(t) {
				return false
			}
			if !n.startAdHoc(t) {
				return false
			}
			t.RecipientIP = n.findPeer(t)
		}
	}
	return true
}

func (n *Network) resetWifi(t *Transfer) {
	if n.Mode == "receiving" || t.Peer == "mac" {
		n.deleteFirewallRule(t)
		n.stopAdHoc(t)
	} else {
		n.runCommand("netsh wlan delete profile name=" + t.SSID)
		// rejoin previous wifi
		t.output(n.runCommand("netsh wlan connect name=" + n.PreviousSSID))
	}
}

func (n *Network) addFirewallRule(t *Transfer) bool {

	execPath, err := os.Executable()
	if err != nil {
		t.output("Failed to get executable path.")
		return false
	}
	fwStr := "netsh advfirewall firewall add rule name=flyingcarpet dir=in action=allow program=" +
		execPath + " enable=yes profile=any localport=3290 protocol=tcp"
	fwSlice := strings.Split(fwStr, " ")
	cmd := exec.Command(fwSlice[0], fwSlice[1:]...)
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	_, err = cmd.CombinedOutput()
	if err != nil {
		t.output("Could not create firewall rule. You must run as administrator to receive. (Press Win+X and then A to start an administrator command prompt.)")
		return false
	}
	// t.output("Firewall rule created.")
	return true
}

func (n *Network) deleteFirewallRule(t *Transfer) {
	fwStr := "netsh advfirewall firewall delete rule name=flyingcarpet"
	t.output(n.runCommand(fwStr))
}

func (n *Network) checkForFile(t *Transfer) bool {
	_, err := os.Stat(t.Filepath)
	if err != nil {
		return false
	}
	return true
}

func (n *Network) runCommand(cmdStr string) (output string) {
	var cmdBytes []byte
	err := errors.New("")
	cmdSlice := strings.Split(cmdStr, " ")
	if len(cmdSlice) > 1 {
		cmd := exec.Command(cmdSlice[0], cmdSlice[1:]...)
		cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
		cmdBytes, err = cmd.CombinedOutput()
	} else {
		cmd := exec.Command(cmdStr)
		cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
		cmdBytes, err = cmd.CombinedOutput()
	}
	if err != nil {
		return err.Error()
	}
	return strings.TrimSpace(string(cmdBytes))
}

func (n *Network) teardown(t *Transfer) {
	// if n.Mode == "receiving" {
	// 	os.Remove(t.Filepath)
	// }
	n.resetWifi(t)
}
