import Foundation
import CoreWLAN

let ssid = "networkname".data(using: .utf8)
print(ssid!)

//let iface = CWWiFiClient.shared().interface()
//let ssid = iface!.ssidData()

print(CWKeychainDeleteWiFiPassword(CWKeychainDomain.system, ssid!))
print(CWKeychainDeleteWiFiPassword(CWKeychainDomain.user, ssid!))