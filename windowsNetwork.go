package main

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
	// "log"
	// "path/filepath"
	"regexp"
	"time"
	"errors"
)

func (w *WindowsNetwork) startAdHoc(t *Transfer) bool {
	// fmt.Println(w.runCommand("netsh winsock reset", "Could not reset network adapter."))
	w.stopAdHoc()
	OutputBox.AppendText("\nSSID: " + t.SSID)
	fmt.Println(w.runCommand("netsh wlan set hostednetwork mode=allow ssid="+t.SSID+" key="+t.Passphrase,
		"Could not set hosted network settings."))
	_, err := exec.Command("netsh", "wlan", "start", "hostednetwork").CombinedOutput()
	if err != nil {
		w.teardown(t)
		OutputBox.AppendText(fmt.Sprintf("\nCould not start hosted network. This computer's wireless card/driver may not support it. %s", err))
		return false
	}
	// fmt.Println(w.runCommand("netsh wlan start hostednetwork", "Could not start hosted network."))
	return true
}

func (w *WindowsNetwork) stopAdHoc() {
	fmt.Println(w.runCommand("netsh wlan stop hostednetwork", "Failed to stop hosted network."))
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
		OutputBox.AppendText("\nWrite error")
		return false
	}
	data := []byte(xmlDoc)
	if _, err = outFile.Write(data); err != nil {
		w.teardown(t)
		OutputBox.AppendText("\nWrite error")
		return false
	}
	defer os.Remove(tmpLoc)

	// reset network adapter
	// fmt.Println(w.runCommand("netsh winsock reset", "Could not reset network adapter."))

	// add profile
	fmt.Println(w.runCommand("netsh wlan add profile filename="+tmpLoc+" user=current", "Could not add wireless profile."))

	// join network
	timeout := JOIN_ADHOC_TIMEOUT
	OutputBox.AppendText("\n")
	for t.SSID != w.getCurrentWifi() {
		if timeout <= 0 {
			OutputBox.AppendText("\nCould not find the ad hoc network within the timeout period.")
			return false
		}
		cmdStr := "netsh wlan connect name=" + t.SSID
		cmdSlice := strings.Split(cmdStr, " ")
		_,cmdErr := exec.Command(cmdSlice[0], cmdSlice[1:]...).CombinedOutput()
		if cmdErr != nil {
			OutputBox.Replace(strings.LastIndex(OutputBox.GetValue(), "\n") + 1, OutputBox.GetLastPosition(), 
				fmt.Sprintf("\nFailed to find the ad hoc network. Trying for %2d more seconds. %s",timeout,cmdErr))
		}
		timeout -= 5
		time.Sleep(time.Second * time.Duration(5))
	}
	// OutputBox.AppendText("\n\n")
	return true
}

func (w *WindowsNetwork) findPeer() (peerIP string) {
	ipPattern, _ := regexp.Compile("\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}")

	// clear arp cache
	w.runCommand("arp -d *", "Could not clear arp cache.")

	// get ad hoc ip
	var ifAddr string
	for !ipPattern.Match([]byte(ifAddr)) {
		// ifAddr = w.runCommand("$(ipconfig | Select-String -Pattern '(?<ipaddr>192\\.168\\.173\\..*)').Matches.Groups[1].Value.Trim()",
		// 	"Could not get ad hoc IP.")
		ifCmd := "$(ipconfig | Select-String -Pattern '(?<ipaddr>192\\.168\\.173\\..*)').Matches.Groups[1].Value.Trim()"
		ifBytes, err := exec.Command("powershell", "-c", ifCmd).CombinedOutput()
		if err != nil {
			OutputBox.AppendText("\nError getting ad hoc IP, retrying.")
		}
		ifAddr = strings.TrimSpace(string(ifBytes))
		// OutputBox.AppendText("\nad hoc IP:" + ifAddr)
		time.Sleep(time.Second * time.Duration(2))
	}
	OutputBox.AppendText("\nStarting findPeer")
	// run arp for that ip
	for !ipPattern.Match([]byte(peerIP)) {

		// peerIP = w.runCommand("$(arp -a -N "+ifAddr+" | Select-String -Pattern '(?<ip>192\\.168\\.173\\.\\d{1,3})' | Select-String -NotMatch '(?<nm>("+
		// 	ifAddr+"|192.168.173.255)\\s)').Matches.Value",
		// 	"Could not get peer IP.")

		peerCmd := "$(arp -a -N "+ifAddr+" | Select-String -Pattern '(?<ip>192\\.168\\.173\\.\\d{1,3})' | Select-String -NotMatch '(?<nm>("+ifAddr+"|192.168.173.255)\\s)').Matches.Value"
		peerBytes, err := exec.Command("powershell", "-c", peerCmd).CombinedOutput()
		if err != nil {
			OutputBox.AppendText("\nError getting ad hoc IP, retrying.")
		}
		peerIP = strings.TrimSpace(string(peerBytes))


		OutputBox.Replace(strings.LastIndex(OutputBox.GetValue(), "\n") + 1, OutputBox.GetLastPosition(), 
			fmt.Sprintf("\npeer IP: %s", peerIP))
		// fmt.Printf("\rpeer IP: %s", peerIP)
		time.Sleep(time.Second * time.Duration(2))
	}
	fmt.Printf("\n")
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

func (w WindowsNetwork) connectToPeer(t *Transfer) bool {
	if w.Mode == "receiving" {
		if !w.addFirewallRule() { return false }
		if !w.startAdHoc(t) { return false }
	} else if w.Mode == "sending" {
		if !w.checkForFile(t) {
			OutputBox.AppendText(fmt.Sprintf("\nCould not find file to send: %s",t.Filepath))
			return false
		}
		if t.Peer == "windows" {
			if !w.joinAdHoc(t) { return false }
			t.RecipientIP = w.findPeer()
		} else if t.Peer == "mac" {
			if !w.addFirewallRule() { return false }
			if !w.startAdHoc(t) { return false }
			OutputBox.AppendText("Ad hoc started, running findPeer")
			t.RecipientIP = w.findPeer()
		}
	}
	return true
}

func (w WindowsNetwork) resetWifi(t *Transfer) {
	if w.Mode == "receiving" || t.Peer == "mac" {
		w.deleteFirewallRule()
		w.stopAdHoc()
	} else {
		w.runCommand("netsh wlan delete profile name=" + t.SSID, "Could not delete ad hoc profile.")
		// rejoin previous wifi
		fmt.Println(w.runCommand("netsh wlan connect name=" + w.PreviousSSID, "Could not join ad hoc network."))
	}
}

func (w WindowsNetwork) addFirewallRule() bool {
	execPath,err := os.Executable()
	if err != nil {
		OutputBox.AppendText("\nFailed to get executable path.")
		return false
	}
	fwStr := "netsh advfirewall firewall add rule name=flyingcarpet dir=in action=allow program=" +
	execPath + " enable=yes profile=any localport=3290 protocol=tcp"
	fwSlice := strings.Split(fwStr, " ")
	_,err = exec.Command(fwSlice[0], fwSlice[1:]...).CombinedOutput()
	if err != nil {
		OutputBox.AppendText("\nCould not create firewall rule. You must run as administrator to receive. (Press Win+X and then A to start an administrator command prompt.)")
		return false
	}
	OutputBox.AppendText("\nFirewall rule created.")
	return true
}

func (w WindowsNetwork) deleteFirewallRule() {
	fwStr := "netsh advfirewall firewall delete rule name=flyingcarpet"
	fmt.Println(w.runCommand(fwStr, "Could not delete firewall rule!"))
}

func (w WindowsNetwork) checkForFile(t *Transfer) bool {
	_,err := os.Stat(t.Filepath)
	if err != nil {
		return false
	}
	return true
}

func (w *WindowsNetwork) runCommand(cmd string, errDesc string) (output string) {
	var cmdBytes []byte
	err := errors.New("")
	cmdSlice := strings.Split(cmd, " ")
	if len(cmdSlice) > 1 {
		cmdBytes, err = exec.Command(cmdSlice[0], cmdSlice[1:]...).CombinedOutput()
	} else {
		cmdBytes, err = exec.Command(cmd).CombinedOutput()
	}
	if err != nil {
		fmt.Printf(errDesc+" Error: %s\n", err)
	}
	return strings.TrimSpace(string(cmdBytes))
}

func (w WindowsNetwork) teardown(t *Transfer) {
	if w.Mode == "receiving" {
		os.Remove(t.Filepath)
	}
	w.resetWifi(t)
}