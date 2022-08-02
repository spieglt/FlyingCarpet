package core

import (
	"encoding/binary"
	"errors"
	"fmt"
	"net"
	"os/exec"
	"strings"
	"time"
)

func (t *Transfer) IsListening() {
	t.Listening = t.Peer == "mac" ||
		t.Peer == "ios" ||
		(t.Peer == "linux" && t.Mode == "receiving")
}

func connectToPeer(t *Transfer, ui UI) (err error) {
	if t.Listening { // hosting ad hoc, listening for connection, showing password
		ui.Output(fmt.Sprintf("Transfer password: %s\nPlease use this password on the other end when prompted to start transfer.\n"+
			"=============================\n", t.Password))
		if err = startAdHoc(t, ui); err != nil {
			return
		}
	} else {
		if err = joinAdHoc(t, ui); err != nil {
			return
		}
		if t.Peer == "linux" {
			t.RecipientIP = findLinux(t)
		} else if t.Peer == "windows" {
			t.RecipientIP = findWindows(t, ui)
		}
	}
	return
}

// TODO: fix this function, add error handling.
func startAdHoc(t *Transfer, ui UI) (err error) {
	// or just:
	// nmcli dev wifi hotspot ssid t.SSID band bg channel 11 password t.Password + t.Password
	// ??
	iface, err := getWifiInterface()
	if err != nil {
		return err
	}
	commands := []string{"nmcli con add type wifi ifname " + iface.Name + " con-name " + t.SSID + " autoconnect yes ssid " + t.SSID,
		"nmcli con modify " + t.SSID + " 802-11-wireless.mode ap 802-11-wireless.band bg ipv4.method shared",
		"nmcli con modify " + t.SSID + " wifi-sec.key-mgmt wpa-psk",
		"nmcli con modify " + t.SSID + " wifi-sec.psk \"" + t.Password + t.Password + "\"",
		"nmcli con up " + t.SSID}
	for _, cmd := range commands {
		out := runCommand(cmd)
		if out != "" {
			ui.Output(out)
		}
	}
	return
}

// TODO: fix this function, add error handling.
func joinAdHoc(t *Transfer, ui UI) (err error) {
	ui.Output("Looking for ad-hoc network " + t.SSID)
	var outBytes []byte
	iface, err := getWifiInterface()
	if err != nil {
		return err
	}
	commands := []string{"nmcli con add type wifi ifname " + iface.Name + " con-name \"" + t.SSID + "\" autoconnect yes ssid \"" + t.SSID + "\"",
		"nmcli con modify \"" + t.SSID + "\" wifi-sec.key-mgmt wpa-psk",
		"nmcli con modify \"" + t.SSID + "\" wifi-sec.psk \"" + t.Password + t.Password + "\"",
		"nmcli con up \"" + t.SSID + "\""}
	for i, cmd := range commands {
		outBytes, err = exec.Command("sh", "-c", cmd).CombinedOutput()
		// ui.Output(fmt.Sprintf("outBytes %d: %s", i, string(outBytes)))
		if err != nil {
			ui.Output(fmt.Sprintf("Error %d: %s", i, err.Error()))
		}
	}
	for string(outBytes)[:5] == "Error" {
		select {
		case <-t.Ctx.Done():
			return errors.New("Exiting joinAdHoc, transfer was canceled")
		default:
			time.Sleep(time.Second * time.Duration(5))
			outBytes, err = exec.Command("sh", "-c", "nmcli con up \""+t.SSID+"\"").CombinedOutput()
			ui.Output(string(outBytes))
			if err != nil {
				ui.Output(fmt.Sprintf("Error joining ad hoc network: %s", err))
			}
		}
	}
	ui.Output(string(outBytes))
	return
}

func resetWifi(t *Transfer, ui UI) {
	command := "nmcli con down \"" + t.SSID + "\""
	ui.Output(runCommand(command))
	command = "nmcli con delete \"" + t.SSID + "\""
	ui.Output(runCommand(command))
	command = "nmcli con up \"" + t.PreviousSSID + "\""
	ui.Output(runCommand(command))
	return
}

func getCurrentWifi(ui UI) (ssid string) {
	command := "nmcli -f active,ssid dev wifi | awk '/^yes/{print $2}"
	ssid = runCommand(command)
	return
}

func getCurrentUUID() (uuid string) {
	command := "nmcli -f active,uuid con | awk '/^yes/{print $2}'"
	uuid = runCommand(command)
	return
}

func getWifiInterface() (*net.Interface, error) {
	ifaces, err := net.Interfaces()
	if err != nil {
		return nil, err
	}
	for _, i := range ifaces {
		if (len(i.Name) > 1 && i.Name[:2] == "wl") || (len(i.Name) > 2 && i.Name[:3] == "enx") {
			return &i, nil
		}
	}
	return nil, errors.New("could not find WiFi interface")
}

func getIPAddress(iface *net.Interface) (*net.IPNet, error) {
	if iface == nil {
		return nil, errors.New("nil WiFi interface")
	}
	addrs, err := iface.Addrs()
	if err != nil {
		return nil, errors.New("could not get address for WiFi interface")
	}
	var addr *net.IPNet
	for _, a := range addrs {
		if ipnet, ok := a.(*net.IPNet); ok {
			if ip4 := ipnet.IP.To4(); ip4 != nil {
				addr = &net.IPNet{
					IP:   ip4,
					Mask: ipnet.Mask[len(ipnet.Mask)-4:],
				}
				break
			}
		}
	}
	return addr, nil
}

func getBroadcast(n *net.IPNet) string {
	num := binary.BigEndian.Uint32([]byte(n.IP))
	mask := binary.BigEndian.Uint32([]byte(n.Mask))
	num |= ^mask
	addr := net.IPv4(byte(num>>24), byte((num>>16)&0xFF), byte((num>>8)&0xFF), byte(num&0xFF))
	return addr.String()
}

func findMac(t *Transfer, ui UI) (peerIP string, err error) {

	iface, err := getWifiInterface()
	for err != nil || iface == nil {
		select {
		case <-t.Ctx.Done():
			return "", errors.New("Exiting findMac, transfer canceled")
		default:
			ui.Output("Looking for interface...")
			time.Sleep(time.Duration(1) * time.Second)
			iface, err = getWifiInterface()
		}
	}

	currentNetwork, err := getIPAddress(iface)
	for err != nil || currentNetwork == nil {
		select {
		case <-t.Ctx.Done():
			return "", errors.New("Exiting findMac, transfer canceled")
		default:
			ui.Output("Looking for address...")
			time.Sleep(time.Duration(1) * time.Second)
			currentNetwork, err = getIPAddress(iface)
		}
	}

	currentIP := strings.Split(currentNetwork.String(), "/")[0] // strip CIDR subnet
	broadcast := getBroadcast(currentNetwork)
	pingString := "ping -b -c 5 " + broadcast + " 2>&1" + // ping broadcast address, include stderr
		" | grep --line-buffered -oE '[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}'" + // get all IPs
		" | grep --line-buffered -vE " + broadcast + // exclude broadcast address
		" | grep -vE '" + currentIP + "$'" // exclude current IP

	ui.Output("Looking for peer IP.")
	for peerIP == "" {
		select {
		case <-t.Ctx.Done():
			return "", errors.New("Exiting findMac, transfer canceled")
		default:
			pingBytes, pingErr := exec.Command("sh", "-c", pingString).CombinedOutput()
			if pingErr != nil {
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

func findWindows(t *Transfer, ui UI) string {
	iface, err := getWifiInterface()
	for err != nil || iface == nil {
		select {
		case <-t.Ctx.Done():
			return ""
		default:
			ui.Output("Looking for interface...")
			time.Sleep(time.Duration(1) * time.Second)
			iface, err = getWifiInterface()
		}
	}

	currentNetwork, err := getIPAddress(iface)
	for err != nil || currentNetwork == nil {
		select {
		case <-t.Ctx.Done():
			return ""
		default:
			ui.Output("Looking for address...")
			time.Sleep(time.Duration(1) * time.Second)
			currentNetwork, err = getIPAddress(iface)
		}
	}
	addr := currentNetwork.String()
	currentIP := strings.Split(addr, "/")[0]
	if strings.Contains(currentIP, "192.168.137") {
		return "192.168.137.1"
	}
	return "192.168.173.1"
}

func findLinux(t *Transfer) string {
	return "10.42.0.1"
}

func runCommand(cmd string) (output string) {
	cmdBytes, err := exec.Command("sh", "-c", cmd).CombinedOutput()
	if err != nil {
		return err.Error()
	}
	return strings.TrimSpace(string(cmdBytes))
}

// WriteDLL is a stub for a function only needed on Windows
func WriteDLL() (string, error) { return "", nil }
