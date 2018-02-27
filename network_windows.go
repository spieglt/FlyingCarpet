package main

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"syscall"
	"time"
)

func connectToPeer(t *Transfer) (err error) {
	if t.Mode == "receiving" {
		if err = addFirewallRule(t); err != nil {
			return
		}
		if err = startAdHoc(t); err != nil {
			return
		}
	} else if t.Mode == "sending" {
		if t.Peer == "windows" {
			if err = joinAdHoc(t); err != nil {
				return
			}
			t.RecipientIP, err = findPeer(t)
			if err != nil {
				return
			}
		} else if t.Peer == "mac" || t.Peer == "linux" {
			if err = addFirewallRule(t); err != nil {
				return
			}
			if err = startAdHoc(t); err != nil {
				return
			}
			t.RecipientIP, err = findPeer(t)
			if err != nil {
				return
			}
		}
	}
	return
}

func startAdHoc(t *Transfer) (err error) {

	runCommand("netsh winsock reset")
	runCommand("netsh wlan stop hostednetwork")
	t.output("SSID: " + t.SSID)
	runCommand("netsh wlan set hostednetwork mode=allow ssid=" + t.SSID + " key=" + t.Passphrase + t.Passphrase)
	cmd := exec.Command("netsh", "wlan", "start", "hostednetwork")
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	_, err = cmd.CombinedOutput()
	if err == nil {
		t.AdHocCapable = true
		return
		// TODO: replace with "echo %errorlevel%" == "1"
	} else if err.Error() == "exit status 1" {
		t.output("Could not start hosted network, trying Wi-Fi Direct.")
		t.AdHocCapable = false

		go startLegacyAP(t)
		if msg := <-t.WfdRecvChan; msg != "started" {
			return errors.New("Could not start Wi-Fi Direct: " + msg)
		}
		return nil
	} else {
		return errors.New("Could not start hosted network: " + err.Error())
	}
}

func stopAdHoc(t *Transfer) {
	if t.AdHocCapable {
		t.output(runCommand("netsh wlan stop hostednetwork"))
	} else {
		t.output("Stopping Wi-Fi Direct.")
		// blocking here, running this twice
		timeChan := make(chan int)
		go func() {
			time.Sleep(time.Second * 2)
			timeChan <- 0
		}()
		select {
		case t.WfdSendChan <- "quit":
			t.output("Sent quit")
			reply := <-t.WfdRecvChan
			t.output("Wi-Fi Direct says: " + reply)
		case <-timeChan:
			t.output("Wi-Fi Direct did not respond to quit request, is likely not running.")
		}
		close(t.WfdSendChan)
	}
}

func joinAdHoc(t *Transfer) (err error) {
	cmd := exec.Command("cmd", "/C", "echo %USERPROFILE%")
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	cmdBytes, err := cmd.CombinedOutput()
	if err != nil {
		return errors.New("Error getting temp location." + err.Error())
	}
	tmpLoc := strings.TrimSpace(string(cmdBytes)) + "\\AppData\\Local\\Temp\\adhoc.xml"

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
		return errors.New("Write error: " + err.Error())
	}
	data := []byte(xmlDoc)
	if _, err = outFile.Write(data); err != nil {
		return errors.New("Write error: " + err.Error())
	}
	defer os.Remove(tmpLoc)

	// add profile
	t.output(runCommand("netsh wlan add profile filename=" + tmpLoc + " user=current"))

	// join network
	t.output("Looking for ad-hoc network " + t.SSID + " for " + strconv.Itoa(joinAdHocTimeout) + " seconds...")
	timeout := joinAdHocTimeout
	for t.SSID != getCurrentWifi(t) {
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting joinAdHoc, transfer was canceled.")
		default:
			if timeout <= 0 {
				return errors.New("Could not find the ad hoc network within " + strconv.Itoa(joinAdHocTimeout) + " seconds.")
			}
			cmdStr := "netsh wlan connect name=" + t.SSID
			cmdSlice := strings.Split(cmdStr, " ")
			joinCmd := exec.Command(cmdSlice[0], cmdSlice[1:]...)
			joinCmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
			/*_, cmdErr :=*/ joinCmd.CombinedOutput()
			// if cmdErr != nil {
			// 	t.output(fmt.Sprintf("Failed to find the ad hoc network. Trying for %2d more seconds. %s", timeout, cmdErr))
			// }
			timeout -= 3
			time.Sleep(time.Second * time.Duration(3))
		}
	}
	return
}

func findPeer(t *Transfer) (string, error) {
	ipPattern, _ := regexp.Compile("\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}")

	// clear arp cache
	runCommand("arp -d *")

	// get ad hoc ip
	var ifAddr string
	for !ipPattern.Match([]byte(ifAddr)) {
		ifString := "$(ipconfig | Select-String -Pattern '(?<ipaddr>192\\.168\\.(137|173)\\..*)').Matches.Groups[2].Value.Trim()"
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
	var peerIP string
	t.output("Looking for peer IP...")
	for !ipPattern.Match([]byte(peerIP)) {
		select {
		case <-t.Ctx.Done():
			return "", errors.New("Exiting joinAdHoc, transfer was canceled.")
		default:
			peerString := "$(arp -a -N " + ifAddr + " | Select-String -Pattern '(?<ip>192\\.168\\." + thirdOctet + "\\.\\d{1,3})' | Select-String -NotMatch '(?<nm>(" + ifAddr + "|192.168." + thirdOctet + ".255)\\s)').Matches.Value"
			peerCmd := exec.Command("powershell", "-c", peerString)
			peerCmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
			peerBytes, err := peerCmd.CombinedOutput()
			if err != nil {
				t.output("Error getting peer IP, retrying.")
			}
			peerIP = strings.TrimSpace(string(peerBytes))
			time.Sleep(time.Second * time.Duration(2))
		}
	}
	t.output(fmt.Sprintf("Peer IP found: %s", peerIP))
	return peerIP, nil
}

func getCurrentWifi(t *Transfer) (SSID string) {
	cmdStr := "$(netsh wlan show interfaces | Select-String -Pattern 'Profile *: (?<profile>.*)').Matches.Groups[1].Value.Trim()"
	cmd := exec.Command("powershell", "-c", cmdStr)
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	cmdBytes, err := cmd.CombinedOutput()
	if err != nil {
		t.output("Error getting current SSID: " + err.Error())
	}
	SSID = strings.TrimSpace(string(cmdBytes))
	return
}

func getWifiInterface() string {
	return ""
}

func resetWifi(t *Transfer) {
	if t.Mode == "receiving" || t.Peer == "mac" || t.Peer == "linux" {
		deleteFirewallRule(t)
		stopAdHoc(t)
	} else { // if Mode == "sending" && t.Peer == "windows"
		runCommand("netsh wlan delete profile name=" + t.SSID)
		// rejoin previous wifi
		t.output(runCommand("netsh wlan connect name=" + t.PreviousSSID))
	}
}

func addFirewallRule(t *Transfer) (err error) {
	execPath, err := os.Executable()
	if err != nil {
		return errors.New("Failed to get executable path: " + err.Error())
	}
	cmd := exec.Command("netsh", "advfirewall", "firewall", "add", "rule", "name="+filepath.Base(execPath), "dir=in",
		"action=allow", "program="+execPath, "enable=yes", "profile=any", "localport=3290", "protocol=tcp")
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	_, err = cmd.CombinedOutput()
	if err != nil {
		return errors.New("Could not create firewall rule. You must run as administrator to receive. (Right-click \"Flying Carpet.exe\" and select \"Run as administrator.\") " + err.Error())
	}
	// t.output("Firewall rule created.")
	return
}

func deleteFirewallRule(t *Transfer) {
	execPath, err := os.Executable()
	if err != nil {
		t.output("Failed to get executable path: " + err.Error())
	}
	cmd := exec.Command("netsh", "advfirewall", "firewall", "delete", "rule", "name="+filepath.Base(execPath))
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	result, err := cmd.CombinedOutput()
	if err != nil {
		t.output("Could not create firewall rule. You must run as administrator to receive. (Right-click \"Flying Carpet.exe\" and select \"Run as administrator.\") " + err.Error())
	}
	t.output(string(result))
}

func runCommand(cmdStr string) (output string) {
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

func getCurrentUUID(t *Transfer) (uuid string) { return "" }
