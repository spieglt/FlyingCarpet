package main

/*
#cgo CFLAGS: -x objective-c
#cgo LDFLAGS: -framework Foundation
#cgo LDFLAGS: -framework CoreWLAN
#import <Foundation/Foundation.h>
#import <CoreWLAN/CoreWLAN.h>

int startAdHoc(char * cSSID, char * cPassword) {
	NSString * SSID = [[NSString alloc] initWithUTF8String:cSSID];
	NSString * password = [[NSString alloc] initWithUTF8String:cPassword];
	CWInterface * iface = CWWiFiClient.sharedWiFiClient.interface;
	NSError *ibssErr = nil;
	BOOL result = [iface startIBSSModeWithSSID:[SSID dataUsingEncoding:NSUTF8StringEncoding] security:kCWIBSSModeSecurityNone channel:11 password:password error:&ibssErr];
	// NSLog(@"%d", result);
	return result;
}

int joinAdHoc(char * cSSID, char * cPassword) {
	NSString * SSID = [[NSString alloc] initWithUTF8String:cSSID];
	NSString * password = [[NSString alloc] initWithUTF8String:cPassword];
	CWInterface * iface = CWWiFiClient.sharedWiFiClient.interface;
	NSError * ibssErr = nil;
	NSSet<CWNetwork *> * network = [iface scanForNetworksWithName:SSID error:&ibssErr];
	BOOL result = [iface associateToNetwork:network.anyObject password:password error:&ibssErr];
	NSLog(@"%d", result);
	return result;
}
*/
import "C"
import (
	"fmt"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"
	"unsafe"
)

func (n *Network) startAdHoc(t *Transfer) bool {

	ssid := C.CString(t.SSID)
	password := C.CString(t.Passphrase + t.Passphrase)
	var cRes C.int = C.startAdHoc(ssid, password)
	res := int(cRes)

	C.free(unsafe.Pointer(ssid))
	C.free(unsafe.Pointer(password))

	if res == 1 {
		t.output("SSID " + t.SSID + " started.")
		return true
	} else {
		t.output("Failed to start ad hoc network.")
		return false
	}
}

func (n *Network) joinAdHoc(t *Transfer) bool {
	t.output("Looking for ad-hoc network " + t.SSID + " for " + strconv.Itoa(JOIN_ADHOC_TIMEOUT) + " seconds...")
	timeout := JOIN_ADHOC_TIMEOUT
	ssid := C.CString(t.SSID)
	password := C.CString(t.Passphrase + t.Passphrase)
	var cRes C.int = C.joinAdHoc(ssid, password)
	res := int(cRes)

	for res == 0 {
		if timeout <= 0 {
			t.output("Could not find the ad hoc network within " + strconv.Itoa(JOIN_ADHOC_TIMEOUT) + " seconds.")
			return false
		}
		// t.output(fmt.Sprintf("Failed to join the ad hoc network. Trying for %2d more seconds.", timeout))
		timeout -= 5
		time.Sleep(time.Second * time.Duration(5))
		res = int(C.joinAdHoc(ssid, password))
	}
	return true
}

func (n *Network) getCurrentWifi(t *Transfer) (SSID string) {
	cmdStr := "/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport -I | awk '/ SSID/ {print substr($0, index($0, $2))}'"
	SSID = n.runCommand(cmdStr)
	return
}

func (n *Network) getWifiInterface() string {
	getInterfaceString := "networksetup -listallhardwareports | awk '/Wi-Fi/{getline; print $2}'"
	return n.runCommand(getInterfaceString)
}

func (n *Network) getIPAddress(t *Transfer) string {
	var currentIP string
	t.output("Waiting for local IP...")
	for currentIP == "" {
		currentIPString := "ipconfig getifaddr " + n.getWifiInterface()
		currentIPBytes, err := exec.Command("sh", "-c", currentIPString).CombinedOutput()
		if err != nil {

			time.Sleep(time.Second * time.Duration(4))
			continue
		}
		currentIP = strings.TrimSpace(string(currentIPBytes))
	}
	t.output(fmt.Sprintf("Wi-Fi interface IP found: %s", currentIP))
	return currentIP
}

func (n *Network) findMac(t *Transfer) (peerIP string, success bool) {
	timeout := FIND_MAC_TIMEOUT
	currentIP := n.getIPAddress(t)
	pingString := "ping -c 5 169.254.255.255 | " + // ping broadcast address
		"grep --line-buffered -oE '[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}' | " + // get all IPs
		"grep --line-buffered -vE '169.254.255.255' | " + // exclude broadcast address
		"grep -vE '" + currentIP + "'" // exclude current IP

	t.output("Looking for peer IP for " + strconv.Itoa(FIND_MAC_TIMEOUT) + " seconds.")
	for peerIP == "" {
		if timeout <= 0 {
			t.output("Could not find the peer computer within " + strconv.Itoa(FIND_MAC_TIMEOUT) + " seconds.")
			return "", false
		}
		pingBytes, pingErr := exec.Command("sh", "-c", pingString).CombinedOutput()
		if pingErr != nil {
			// t.output(fmt.Sprintf("Could not find peer. Waiting %2d more seconds. %s", timeout, pingErr))
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

func (n *Network) findWindows(t *Transfer) (peerIP string) {
	currentIP := n.getIPAddress(t)
	if strings.Contains(currentIP, "192.168.137") {
		return "192.168.137.1"
	} else {
		return "192.168.173.1"
	}
}

func (n *Network) findLinux(t *Transfer) (peerIP string, success bool) {
	// timeout := FIND_MAC_TIMEOUT
	// currentIP := n.getIPAddress(t)
	// pingString := "ping -b -c 5 $(ifconfig | awk '/" + n.getWifiInterface() + "/ {for(i=1; i<=3; i++) {getline;}; print $6}') 2>&1 | " + // ping broadcast address
	// 	"grep --line-buffered -oE '[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}' | " + // get all IPs
	// 	"grep --line-buffered -vE $(ifconfig | awk '/" + n.getWifiInterface() + "/ {for(i=1; i<=3; i++) {getline;}; print $6}') | " + // exclude broadcast address
	// 	"grep -vE '" + currentIP + "'" // exclude current IP

	// t.output("Looking for peer IP for " + strconv.Itoa(FIND_MAC_TIMEOUT) + " seconds.")
	// for peerIP == "" {
	// 	if timeout <= 0 {
	// 		t.output("Could not find the peer computer within " + strconv.Itoa(FIND_MAC_TIMEOUT) + " seconds.")
	// 		return "", false
	// 	}
	// 	pingBytes, pingErr := exec.Command("sh", "-c", pingString).CombinedOutput()
	// 	if pingErr != nil {
	// 		t.output(fmt.Sprintf("Could not find peer. Waiting %2d more seconds. %s", timeout, pingErr))
	// 		t.output(fmt.Sprintf("peer IP: %s",string(pingBytes)))
	// 		timeout -= 2
	// 		time.Sleep(time.Second * time.Duration(2))
	// 		continue
	// 	}
	// 	peerIPs := string(pingBytes)
	// 	peerIP = peerIPs[:strings.Index(peerIPs, "\n")]
	// }
	// t.output(fmt.Sprintf("Peer IP found: %s", peerIP))
	// success = true
	// return
	return "10.42.0.1", true
}

func (n *Network) connectToPeer(t *Transfer) bool {

	if n.Mode == "sending" {
		if !n.checkForFile(t) {
			t.output(fmt.Sprintf("Could not find file to send: %s", t.Filepath))
			return false
		}
		if !n.joinAdHoc(t) {
			return false
		}
		go n.stayOnAdHoc(t)
		if t.Peer == "mac" {
			var ok bool
			t.RecipientIP, ok = n.findMac(t)
			if !ok {
				return false
			}
		} else if t.Peer == "windows" {
			t.RecipientIP = n.findWindows(t)
		} else if t.Peer == "linux" {
			ip, ok := n.findLinux(t)
			t.RecipientIP = ip
			if !ok {
				return false
			}
		}
	} else if n.Mode == "receiving" {
		if t.Peer == "windows" || t.Peer == "linux" {
			if !n.joinAdHoc(t) {
				return false
			}
			go n.stayOnAdHoc(t)
		} else if t.Peer == "mac" {
			if !n.startAdHoc(t) {
				return false
			}
		}
	}
	return true
}

func (n *Network) resetWifi(t *Transfer) {
	wifiInterface := n.getWifiInterface()
	cmdString := "networksetup -setairportpower " + wifiInterface + " off && networksetup -setairportpower " + wifiInterface + " on"
	t.output(n.runCommand(cmdString))
	if t.Peer == "windows" || n.Mode == "sending" {
		cmdString = "networksetup -removepreferredwirelessnetwork " + wifiInterface + " " + t.SSID
		t.output(n.runCommand(cmdString))
	}
}

func (n *Network) stayOnAdHoc(t *Transfer) {

	for {
		select {
		case <-t.AdHocChan:
			t.output("Stopping ad hoc connection.")
			t.AdHocChan <- true
			return
		default:
			if n.getCurrentWifi(t) != t.SSID {
				n.joinAdHoc(t)
			}
			time.Sleep(time.Second * 1)
		}
	}
}

func (n *Network) checkForFile(t *Transfer) bool {
	_, err := os.Stat(t.Filepath)
	if err != nil {
		return false
	}
	return true
}

func (n *Network) runCommand(cmd string) (output string) {
	cmdBytes, err := exec.Command("sh", "-c", cmd).CombinedOutput()
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

func (n *Network) getCurrentUUID(t *Transfer) (uuid string) { return "" }
