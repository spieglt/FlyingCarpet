import Foundation
import CoreWLAN

if let iface = CWWiFiClient.shared().interface() {
	let nets: Set<CWNetwork>
	do {
		try nets = iface.scanForNetworks(withName: "networkname")
		print(nets)
		try iface.associate(to: nets.first!, password: "networkpassword")
	} catch let error as NSError {
		print("Error", error)
		exit(1)
	}
} else {
	print("Invalid interface")
	exit(1)
}