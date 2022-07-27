package core

import (
	"bytes"
	_ "embed"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"syscall"
	"time"
)

func connectToPeer(t *Transfer, ui UI) (err error) {
	if t.Mode == "receiving" {
		if err = addFirewallRule(t); err != nil {
			return
		}
		if err = startAdHoc(t, ui); err != nil {
			return
		}
	} else if t.Mode == "sending" {
		if t.Peer == "windows" {
			if err = joinAdHoc(t, ui); err != nil {
				return
			}
			t.RecipientIP, err = findPeer(t, ui)
			if err != nil {
				return
			}
		} else if t.Peer == "mac" || t.Peer == "linux" || t.Peer == "ios" {
			if err = addFirewallRule(t); err != nil {
				return
			}
			if err = startAdHoc(t, ui); err != nil {
				return
			}
			t.RecipientIP, err = findPeer(t, ui)
			if err != nil {
				return
			}
		}
	}
	return
}

func startAdHoc(t *Transfer, ui UI) (err error) {
	runCommand("netsh winsock reset")
	runCommand("netsh wlan stop hostednetwork")
	ui.Output("SSID: " + t.SSID)
	runCommand("netsh wlan set hostednetwork mode=allow ssid=" + t.SSID + " key=" + t.Password + t.Password)
	cmd := exec.Command("netsh", "wlan", "start", "hostednetwork")
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	_, err = cmd.CombinedOutput()
	if err == nil {
		t.AdHocCapable = true
		return
		// TODO: replace with "echo %errorlevel%" == "1"
	} else if err.Error() == "exit status 1" {
		ui.Output("Could not start hosted network, trying Wi-Fi Direct.")
		t.AdHocCapable = false

		go startLegacyAP(t, ui)
		if msg := <-t.WfdRecvChan; msg != "started" {
			return errors.New("Could not start Wi-Fi Direct: " + msg)
		}
		return nil
	} else {
		return errors.New("Could not start hosted network: " + err.Error())
	}
}

func stopAdHoc(t *Transfer, ui UI) {
	if t.AdHocCapable {
		ui.Output(runCommand("netsh wlan stop hostednetwork"))
	} else {
		// ui.Output("Stopping Wi-Fi Direct.")
		select {
		case t.WfdSendChan <- "quit":
			/*reply :=*/ <-t.WfdRecvChan
			// ui.Output("Wi-Fi Direct says: " + reply)
		case <-time.After(time.Second * 3):
			ui.Output("Wi-Fi Direct did not respond to quit request, is likely not running.")
		}
		close(t.WfdSendChan)
	}
}

func joinAdHoc(t *Transfer, ui UI) (err error) {
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
		"				<keyMaterial>" + t.Password + t.Password + "</keyMaterial>\r\n" +
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
	ui.Output(runCommand("netsh wlan add profile filename=" + tmpLoc + " user=current"))

	// join network
	ui.Output("Looking for ad-hoc network " + t.SSID)
	for t.SSID != getCurrentWifi(ui) {
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting joinAdHoc, transfer was canceled")
		default:
			cmdStr := "netsh wlan connect name=" + t.SSID
			cmdSlice := strings.Split(cmdStr, " ")
			joinCmd := exec.Command(cmdSlice[0], cmdSlice[1:]...)
			joinCmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
			/*_, cmdErr :=*/ joinCmd.CombinedOutput()
			// if cmdErr != nil {
			// 	ui.Output(fmt.Sprintf("Failed to find the ad hoc network. Trying for %2d more seconds. %s", timeout, cmdErr))
			// }
			time.Sleep(time.Second * time.Duration(3))
		}
	}
	return
}

func findPeer(t *Transfer, ui UI) (string, error) {
	ipPattern, _ := regexp.Compile("\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}")

	// clear arp cache
	runCommand("arp -d *")

	// get ad hoc ip
	var ifAddr string
	for !ipPattern.Match([]byte(ifAddr)) {
		select {
		case <-t.Ctx.Done():
			return "", errors.New("Exiting joinAdHoc, transfer was canceled")
		default:
			ifString := "$(ipconfig | Select-String -Pattern '(?<ipaddr>192\\.168\\.(137|173)\\..*)').Matches.Groups[2].Value.Trim()"
			ifCmd := exec.Command("powershell", "-c", ifString)
			ifCmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
			ifBytes, err := ifCmd.CombinedOutput()
			if err != nil {
				ui.Output("Error getting ad hoc IP, retrying.")
			}
			ifAddr = strings.TrimSpace(string(ifBytes))
			time.Sleep(time.Second * time.Duration(2))
		}
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
	ui.Output("Looking for peer IP...")
	for !ipPattern.Match([]byte(peerIP)) {
		select {
		case <-t.Ctx.Done():
			return "", errors.New("Exiting joinAdHoc, transfer was canceled")
		default:
			peerString := "$(arp -a -N " + ifAddr + " | Select-String -Pattern '(?<ip>192\\.168\\." + thirdOctet + "\\.\\d{1,3})' | Select-String -NotMatch '(?<nm>(" + ifAddr + "|192.168." + thirdOctet + ".255)\\s)').Matches.Value"
			peerCmd := exec.Command("powershell", "-c", peerString)
			peerCmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
			peerBytes, err := peerCmd.CombinedOutput()
			if err != nil {
				ui.Output("Error getting peer IP, retrying.")
			}
			peerIP = strings.TrimSpace(string(peerBytes))
			time.Sleep(time.Second * time.Duration(2))
		}
	}
	ui.Output(fmt.Sprintf("Peer IP found: %s", peerIP))
	return peerIP, nil
}

func getCurrentWifi(ui UI) (SSID string) {
	cmdStr := "$(netsh wlan show interfaces | Select-String -Pattern 'Profile *: (?<profile>.*)').Matches.Groups[1].Value.Trim()"
	cmd := exec.Command("powershell", "-c", cmdStr)
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	cmdBytes, err := cmd.CombinedOutput()
	if err != nil {
		ui.Output("Error getting current SSID: " + err.Error())
	}
	SSID = strings.TrimSpace(string(cmdBytes))
	return
}

func getWifiInterface() string {
	return ""
}

func resetWifi(t *Transfer, ui UI) {
	if t.Mode == "receiving" || t.Peer == "mac" || t.Peer == "linux" || t.Peer == "ios" {
		deleteFirewallRule(ui)
		stopAdHoc(t, ui)
	} else { // if Mode == "sending" && t.Peer == "windows"
		runCommand("netsh wlan delete profile name=" + t.SSID)
		// rejoin previous wifi
		ui.Output(runCommand("netsh wlan connect name=" + t.PreviousSSID))
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
	// ui.Output("Firewall rule created.")
	return
}

func deleteFirewallRule(ui UI) {
	execPath, err := os.Executable()
	if err != nil {
		ui.Output("Failed to get executable path: " + err.Error())
	}
	cmd := exec.Command("netsh", "advfirewall", "firewall", "delete", "rule", "name="+filepath.Base(execPath))
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	result, err := cmd.CombinedOutput()
	if err != nil {
		ui.Output("Could not create firewall rule. You must run as administrator to receive. (Right-click \"Flying Carpet.exe\" and select \"Run as administrator.\") " + err.Error())
	}
	ui.Output(string(result))
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

func getCurrentUUID() (uuid string) { return "" }

//go:embed wfd.dll
var dllFile []byte

// used if running CLI version as the wifi direct
// dll won't have been bundled with the GUI
func WriteDLL() (string, error) {
	var err error
	// find suitable location to write dll and complete filepath
	tempLoc := os.TempDir()
	if tempLoc == "" {
		tempLoc, err = os.Executable()
		if err != nil {
			return "", errors.New("error finding suitable location to write dll: " + err.Error())
		}
	}
	tempLoc = tempLoc + string(os.PathSeparator) + "wfd.dll"

	// delete preexisting dll, create new one, and write it
	os.Remove(tempLoc)
	outputFile, err := os.Create(tempLoc)
	if err != nil {
		return "", errors.New("error creating dll: " + err.Error())
	}
	defer outputFile.Close()
	dllReader := bytes.NewReader(dllFile)
	_, err = io.Copy(outputFile, dllReader)
	if err != nil {
		return "", errors.New("error writing embedded data to output file: " + err.Error())
	}
	return tempLoc, err
}
