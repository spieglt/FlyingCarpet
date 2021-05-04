package core

/*
#cgo CFLAGS: -x objective-c
#cgo LDFLAGS: -framework Foundation
#cgo LDFLAGS: -framework CoreWLAN
#cgo LDFLAGS: -framework SecurityFoundation
#import <Foundation/Foundation.h>
#import <CoreWLAN/CoreWLAN.h>
#import <SecurityFoundation/SFAuthorization.h>

SFAuthorization *auth = nil;

int startAdHoc(char * cSSID, char * cPassword) {
	NSString * SSID = [[NSString alloc] initWithUTF8String:cSSID];
	NSString * password = [[NSString alloc] initWithUTF8String:cPassword];
	CWInterface * iface = CWWiFiClient.sharedWiFiClient.interface;
	NSError *ibssErr = nil;
	BOOL result = [iface startIBSSModeWithSSID:[SSID dataUsingEncoding:NSUTF8StringEncoding] security:kCWIBSSModeSecurityNone channel:11 password:password error:&ibssErr];
	if (!result) {
		NSLog(@"startAdHoc error: %@",ibssErr);
	}
	return result;
}
int joinAdHoc(char * cSSID, char * cPassword) {
	NSString * SSID = [[NSString alloc] initWithUTF8String:cSSID];
	NSString * password = [[NSString alloc] initWithUTF8String:cPassword];
	CWInterface * iface = CWWiFiClient.sharedWiFiClient.interface;
	NSError * ibssErr = nil;
	NSSet<CWNetwork *> * network = [iface scanForNetworksWithName:SSID error:&ibssErr];
	BOOL result = [iface associateToNetwork:network.anyObject password:password error:&ibssErr];
//	if (!result) {
//		NSLog(@"joinAdHoc error: %@",ibssErr);
//	}
	return result;
}
int moveNetworkToTop(char * cSSID) {
	NSString * SSID = [[NSString alloc] initWithUTF8String:cSSID];
	CWInterface *iface = CWWiFiClient.sharedWiFiClient.interface;
	BOOL found = false;
	CWMutableConfiguration *config;
	for (int i = 0; found == false && i < 10; i++) {
		config = [CWMutableConfiguration configurationWithConfiguration:iface.configuration];
		NSMutableArray *networks = [NSMutableArray arrayWithArray: [config.networkProfiles array]];
		for (CWNetworkProfile *profile in [networks copy]) {
			if ([[profile ssid] isEqualToString:SSID]) {
				CWNetworkProfile *tmp = profile;
				[networks removeObject:tmp];
				[networks insertObject:tmp atIndex:0];
				found = true;
			}
		}
		config.networkProfiles = [NSOrderedSet orderedSetWithArray:networks];
		// printf("Waiting...\n");
		sleep(1);
	}
	if (!found) {
		printf("SSID not found in preferred networks list after 10 seconds.");
		return 0;
	}
	NSError *error = nil;
	BOOL result = [iface commitConfiguration:config authorization:auth error:&error];
	if (!result) {
		NSLog(@"Commit configuration error: %@",error);
	}
	return result;
}
int deleteNetwork(char * cSSID) {
	NSString * SSID = [[NSString alloc] initWithUTF8String:cSSID];
	CWInterface *iface = CWWiFiClient.sharedWiFiClient.interface;
	CWMutableConfiguration *config;
	config = [CWMutableConfiguration configurationWithConfiguration:iface.configuration];
	NSMutableArray *networks = [NSMutableArray arrayWithArray: [config.networkProfiles array]];
	for (CWNetworkProfile *profile in [networks copy]) {
		if ([[profile ssid] isEqualToString:SSID]) {
			CWNetworkProfile *tmp = profile;
			[networks removeObject:tmp];
		}
		config.networkProfiles = [NSOrderedSet orderedSetWithArray:networks];
	}
	NSError *error = nil;
	BOOL result = [iface commitConfiguration:config authorization:auth error:&error];
	if (!result) {
		NSLog(@"Delete network error: %@",error);
	}
	return result;
}
int getAuth() {
	auth = [SFAuthorization authorization];
	NSError *error = nil;
	BOOL authResult = [auth obtainWithRight:"system.preferences"
			flags: (kAuthorizationFlagExtendRights |
				kAuthorizationFlagInteractionAllowed |
				kAuthorizationFlagPreAuthorize)
			error:&error
		];
	if (!authResult) {
		NSLog(@"authError: %@", error);
	}
	return authResult;
}
*/
import "C"
import (
	"errors"
	"fmt"
	"os/exec"
	"strings"
	"time"
	"unsafe"
)

func connectToPeer(t *Transfer, ui UI) (err error) {

	if t.Mode == "sending" {
		if err = joinAdHoc(t, ui); err != nil {
			return
		}
		// go stayOnAdHoc(t, ui)
		if t.Peer == "mac" {
			t.RecipientIP, err = findMac(t, ui)
			if err != nil {
				return
			}
		} else if t.Peer == "windows" {
			t.RecipientIP = findWindows(t, ui)
		} else if t.Peer == "linux" {
			t.RecipientIP = findLinux(t)
		}
	} else if t.Mode == "receiving" {
		if t.Peer == "windows" || t.Peer == "linux" {
			if err = joinAdHoc(t, ui); err != nil {
				return
			}
			// go stayOnAdHoc(t, ui)
		} else if t.Peer == "mac" {
			if err = startAdHoc(t, ui); err != nil {
				return
			}
		}
	}
	return
}

func startAdHoc(t *Transfer, ui UI) (err error) {

	ssid := C.CString(t.SSID)
	password := C.CString(t.Password + t.Password)
	var cRes C.int = C.startAdHoc(ssid, password)
	res := int(cRes)

	C.free(unsafe.Pointer(ssid))
	C.free(unsafe.Pointer(password))

	if res == 1 {
		ui.Output("SSID " + t.SSID + " started.")
		return
	}
	return errors.New("Failed to start ad hoc network")
}

func joinAdHoc(t *Transfer, ui UI) (err error) {
	if authRes := int(C.getAuth()); authRes == 0 {
		ui.Output("Error getting authorization")
	}
	ui.Output("Looking for ad-hoc network " + t.SSID)
	ssid := C.CString(t.SSID)
	password := C.CString(t.Password + t.Password)

	var cRes C.int = C.joinAdHoc(ssid, password)
	res := int(cRes)

	for res == 0 {
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting joinAdHoc, transfer was canceled")
		default:
			time.Sleep(time.Second * time.Duration(3))
			res = int(C.joinAdHoc(ssid, password))
		}
	}
	// prefer flyingCarpet network so mac doesn't jump to another
	// cRes = C.moveNetworkToTop(ssid)
	// res = int(cRes)
	// ui.Output(fmt.Sprintf("%s is preferred network: %t", t.SSID, (res != 0)))
	return
}

func getCurrentWifi(ui UI) (SSID string) {
	cmdStr := "/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport -I | awk '/ SSID/ {print substr($0, index($0, $2))}'"
	SSID = runCommand(cmdStr)
	return
}

func getWifiInterface() string {
	getInterfaceString := "networksetup -listallhardwareports | awk '/Wi-Fi/{getline; print $2}'"
	return runCommand(getInterfaceString)
}

func getIPAddress(ui UI) string {
	var currentIP string
	ui.Output("Waiting for local IP...")
	for currentIP == "" {
		currentIPString := "ipconfig getifaddr " + getWifiInterface()
		currentIPBytes, err := exec.Command("sh", "-c", currentIPString).CombinedOutput()
		if err != nil {
			time.Sleep(time.Second * time.Duration(3))
			continue
		}
		currentIP = strings.TrimSpace(string(currentIPBytes))
	}
	ui.Output(fmt.Sprintf("Wi-Fi interface IP found: %s", currentIP))
	return currentIP
}

func findMac(t *Transfer, ui UI) (peerIP string, err error) {
	currentIP := getIPAddress(ui)
	pingString := "ping -c 5 169.254.255.255 | " + // ping broadcast address
		"grep --line-buffered -oE '[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}' | " + // get all IPs
		"grep --line-buffered -vE '169.254.255.255' | " + // exclude broadcast address
		"grep -vE '" + currentIP + "'" // exclude current IP

	ui.Output("Looking for peer IP")
	for peerIP == "" {
		select {
		case <-t.Ctx.Done():
			return "", errors.New("exiting dialPeer, transfer was canceled")
		default:
			pingBytes, pingErr := exec.Command("sh", "-c", pingString).CombinedOutput()
			if pingErr != nil {
				// ui.Output(fmt.Sprintf("Could not find peer. Waiting %2d more seconds. %s", timeout, pingErr))
				time.Sleep(time.Second * time.Duration(2))
				continue
			}
			peerIPs := string(pingBytes)
			peerIP = peerIPs[:strings.Index(peerIPs, "\n")]
		}
	}
	ui.Output(fmt.Sprintf("Peer IP found: %s", peerIP))
	return
}

func findWindows(t *Transfer, ui UI) (peerIP string) {
	currentIP := getIPAddress(ui)
	if strings.Contains(currentIP, "192.168.137") {
		return "192.168.137.1"
	}
	return "192.168.173.1"
}

func findLinux(t *Transfer) (peerIP string) {
	// timeout := findMacTimeout
	// currentIP := getIPAddress(t)
	// pingString := "ping -b -c 5 $(ifconfig | awk '/" + getWifiInterface() + "/ {for(i=1; i<=3; i++) {getline;}; print $6}') 2>&1 | " + // ping broadcast address
	// 	"grep --line-buffered -oE '[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}' | " + // get all IPs
	// 	"grep --line-buffered -vE $(ifconfig | awk '/" + getWifiInterface() + "/ {for(i=1; i<=3; i++) {getline;}; print $6}') | " + // exclude broadcast address
	// 	"grep -vE '" + currentIP + "'" // exclude current IP

	// ui.Output("Looking for peer IP for " + strconv.Itoa(findMacTimeout) + " seconds.")
	// for peerIP == "" {
	// 	if timeout <= 0 {
	// 		ui.Output("Could not find the peer computer within " + strconv.Itoa(findMacTimeout) + " seconds.")
	// 		return "", false
	// 	}
	// 	pingBytes, pingErr := exec.Command("sh", "-c", pingString).CombinedOutput()
	// 	if pingErr != nil {
	// 		ui.Output(fmt.Sprintf("Could not find peer. Waiting %2d more seconds. %s", timeout, pingErr))
	// 		ui.Output(fmt.Sprintf("peer IP: %s",string(pingBytes)))
	// 		timeout -= 2
	// 		time.Sleep(time.Second * time.Duration(2))
	// 		continue
	// 	}
	// 	peerIPs := string(pingBytes)
	// 	peerIP = peerIPs[:strings.Index(peerIPs, "\n")]
	// }
	// ui.Output(fmt.Sprintf("Peer IP found: %s", peerIP))
	// success = true
	// return
	return "10.42.0.1"
}

func resetWifi(t *Transfer, ui UI) {
	wifiInterface := getWifiInterface()
	cmdString := "networksetup -setairportpower " + wifiInterface + " off && networksetup -setairportpower " + wifiInterface + " on"
	ui.Output(runCommand(cmdString))
	if t.Peer == "windows" || t.Peer == "linux" || t.Mode == "sending" {
		// cmdString = "networksetup -removepreferredwirelessnetwork " + wifiInterface + " " + t.SSID
		// ui.Output(runCommand(cmdString) + " (If you did not enter password at prompt, SSID will not be removed from your System keychain or preferred networks list.)")
		if shouldPrompt := ui.ShowPwPrompt(); shouldPrompt {
			res := int(C.deleteNetwork(C.CString(t.SSID)))
			if res == 0 {
				ui.Output("Error removing " + t.SSID + " from preferred wireless networks list.")
			} else {
				ui.Output("Removed " + t.SSID + " from preferred wireless networks list.")
			}
		}
	}
}

func stayOnAdHoc(t *Transfer, ui UI) {
	for {
		select {
		case <-t.Ctx.Done():
			ui.Output("Stopping ad hoc connection.")
			return
		default:
			time.Sleep(time.Second * 5)
			if getCurrentWifi(ui) != t.SSID {
				joinAdHoc(t, ui)
			}
		}
	}
}

func runCommand(cmd string) (output string) {
	cmdBytes, err := exec.Command("sh", "-c", cmd).CombinedOutput()
	if err != nil {
		return err.Error()
	}
	return strings.TrimSpace(string(cmdBytes))
}

func getCurrentUUID() (uuid string) { return "" }

// WriteDLL is a stub for a function only needed on Windows
func WriteDLL() (string, error) { return "", nil }
