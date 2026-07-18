use crate::error::{fc_error, FCError};
use crate::utils::run_command;
use crate::{InterfaceInfo, Mode, Peer, PeerResource, WiFiInterface, UI};
use tokio::task;

// stub
pub struct WindowsHotspot {
    _inner: (),
}

pub fn is_hosting(peer: &Peer, mode: &Mode) -> bool {
    match peer {
        Peer::Android | Peer::IOS | Peer::MacOS => true,
        Peer::Windows => false,
        Peer::Linux => match mode {
            Mode::Send(_) => false,
            Mode::Receive(_) => true,
        },
    }
}

pub async fn connect_to_peer<T: UI>(
    peer: Peer,
    mode: Mode,
    ssid: String,
    password: String,
    interface: WiFiInterface,
    ui: &T,
) -> Result<PeerResource, FCError> {
    if is_hosting(&peer, &mode) {
        // start hotspot
        ui.output(&format!("Starting hotspot {}", ssid));
        start_hotspot(&ssid, &password, &interface.0)?;
        Ok(PeerResource::LinuxHotspot)
    } else {
        // join hotspot and find gateway
        ui.output(&format!("Joining hotspot {}", ssid));
        join_hotspot(&ssid, &password, &interface.0, ui).await?;
        loop {
            // println!("looking for gateway");
            task::yield_now().await;
            match find_gateway(&interface.0) {
                Ok(gateway) => {
                    if gateway != "" {
                        return Ok(PeerResource::WifiClient(gateway));
                    }
                }
                Err(e) => Err(e)?,
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }
    }
}

fn start_hotspot(ssid: &str, password: &str, interface: &str) -> Result<(), FCError> {
    let nmcli = "nmcli";
    let user_str = &format!("user:{}", get_username());
    let commands = vec![
        vec![
            "con",
            "add",
            "type",
            "wifi",
            "ifname",
            &interface,
            "con-name",
            ssid,
            "autoconnect",
            "yes",
            "ssid",
            ssid,
            "connection.permissions",
            &user_str,
        ],
        vec![
            "con",
            "modify",
            ssid,
            "802-11-wireless.mode",
            "ap",
            "ipv4.method",
            "shared",
        ],
        vec!["con", "modify", ssid, "wifi-sec.key-mgmt", "wpa-psk"],
        // disable Protected Management Frames, which disables WPA3/SAE, which is necessary for M1 Macs to join Linux
        vec!["con", "modify", ssid, "wifi-sec.pmf", "disable"],
        // use AES, not TKIP
        vec!["con", "modify", ssid, "wifi-sec.pairwise", "ccmp"],
        vec!["con", "modify", ssid, "wifi-sec.group", "ccmp"],
        // use WPA2, not WPA
        vec!["con", "modify", ssid, "wifi-sec.proto", "rsn"],
        vec!["con", "modify", ssid, "wifi-sec.psk", password],
        vec!["con", "up", ssid],
    ];
    for command in commands {
        let res = run_command(nmcli, Some(command))?;
        if !res.status.success() {
            let stderr = String::from_utf8_lossy(&res.stderr);
            fc_error(&format!("Could not start hotspot: {}", stderr))?;
        }
        // println!("output: {}", String::from_utf8_lossy(&res.stdout));
    }
    Ok(())
}

// Deletes leftover flyingCarpet_* NetworkManager connections from previous runs that
// crashed or were killed before stop_hotspot() could run (#51). Both hosting and
// joining create a connection named after the SSID, which is always "flyingCarpet_"
// plus 4 hex characters, so anything with that prefix is ours. Returns the names of
// the connections deleted.
pub fn cleanup_stale_connections() -> Result<Vec<String>, FCError> {
    let output = run_command(
        "nmcli",
        Some(vec!["-t", "-f", "NAME,TYPE", "connection", "show"]),
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        fc_error(&format!(
            "Could not list NetworkManager connections: {}",
            stderr
        ))?;
    }
    let mut deleted = vec![];
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        // terse format is NAME:TYPE; the name can't contain a colon (nmcli escapes
        // them, and ours never do), so split on the last one
        let Some((name, connection_type)) = line.rsplit_once(':') else {
            continue;
        };
        if connection_type == "802-11-wireless" && name.starts_with("flyingCarpet_") {
            let delete = run_command("nmcli", Some(vec!["connection", "delete", name]))?;
            if delete.status.success() {
                deleted.push(name.to_string());
            }
        }
    }
    Ok(deleted)
}

pub fn stop_hotspot(
    _peer_resource: Option<&PeerResource>,
    ssid: Option<&str>,
) -> Result<String, FCError> {
    if ssid.is_some() {
        let list = run_command("nmcli", Some(vec!["connection", "show"]))?;
        if String::from_utf8_lossy(&list.stdout).contains(ssid.unwrap()) {
            let options = Some(vec!["connection", "delete", ssid.unwrap()]);
            let command_output = run_command("nmcli", options)?;
            if !command_output.status.success() {
                let stderr = String::from_utf8_lossy(&command_output.stderr);
                fc_error(&format!("Error stopping hotspot: {}", stderr))?;
            }
            let output = String::from_utf8_lossy(&command_output.stdout);
            Ok(format!("Stop hotspot output: {}", output))
        } else {
            Ok(format!("SSID {} was not a known network", ssid.unwrap()))
        }
    } else {
        Ok(String::new())
    }
}

async fn join_hotspot<T: UI>(
    ssid: &str,
    password: &str,
    interface: &str,
    ui: &T,
) -> Result<(), FCError> {
    let nmcli = "nmcli";
    let user_str = &format!("user:{}", get_username());
    let commands = vec![
        vec![
            "con",
            "add",
            "type",
            "wifi",
            "ifname",
            &interface,
            "con-name",
            ssid,
            "autoconnect",
            "yes",
            "ssid",
            ssid,
            "connection.permissions",
            &user_str,
        ],
        vec!["con", "modify", ssid, "wifi-sec.key-mgmt", "wpa-psk"],
        vec!["con", "modify", ssid, "wifi-sec.psk", password],
    ];
    for command in commands {
        let res = run_command(nmcli, Some(command))?;
        if !res.status.success() {
            let stderr = String::from_utf8_lossy(&res.stderr);
            fc_error(&format!("Error joining hotspot: {}", stderr))?;
        }
        // println!(
        //     "join hotspot output: {}",
        //     String::from_utf8_lossy(&res.stdout)
        // );
    }
    loop {
        let res = run_command(nmcli, Some(vec!["con", "up", ssid]))?;
        if !res.status.success() {
            let stderr = String::from_utf8_lossy(&res.stderr);
            // Err(format!("Error joining hotspot: {}", stderr))?;
            let err_msg = format!("Error joining hotspot: {}. Retrying.", stderr);
            ui.output(&err_msg);
            println!("{}", err_msg);
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        } else {
            break;
        }
    }
    Ok(())
}

pub fn get_wifi_interfaces() -> Result<Vec<InterfaceInfo>, FCError> {
    let command = "nmcli";
    let options = vec!["-t", "device"];
    let command_output = run_command(command, Some(options))?;
    let output = String::from_utf8_lossy(&command_output.stdout);
    let mut interfaces: Vec<InterfaceInfo> = vec![];
    for line in output.lines() {
        // Format: DEVICE:TYPE:STATE:CONNECTION
        let split_line: Vec<&str> = line.split(':').collect();
        if split_line.len() < 2 || split_line[1] != "wifi" {
            continue;
        }
        // ip is best-effort: hosting a hotspot doesn't require a connection
        let name = split_line[0].to_string();
        let ip = interface_ipv4(&name);
        interfaces.push(InterfaceInfo {
            name,
            guid: String::new(),
            ip,
        });
    }
    Ok(interfaces)
}

/// Returns the interface's usable IPv4 address as text, or None if it has none.
/// Link-local (169.254.x) addresses mean there's no real network.
fn interface_ipv4(interface_name: &str) -> Option<String> {
    let iface = WiFiInterface(interface_name.to_string(), String::new());
    let cidr = get_ip_cidr(&iface).ok()?;
    let ip = cidr.split('/').next()?.trim().to_string();
    if ip.is_empty() || ip.starts_with("169.254.") {
        None
    } else {
        Some(ip)
    }
}

/// Name of the interface owning the default route, if any (for preselection).
fn default_route_interface() -> Option<String> {
    let output = run_command("sh", Some(vec!["-c", "ip -4 route show default"])).ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Format: "default via 192.168.1.1 dev eth0 proto dhcp ..."
    let tokens: Vec<&str> = stdout.split_whitespace().collect();
    let dev_idx = tokens.iter().position(|&t| t == "dev")?;
    tokens.get(dev_idx + 1).map(|s| s.to_string())
}

fn find_gateway(interface: &str) -> Result<String, FCError> {
    let route_command = format!(
        "route -n | grep {} | grep UG | awk '{{print $2}}'",
        interface
    ); // TODO: not the best but it will do? use regex in rust?
    let output = run_command("sh", Some(vec!["-c", &route_command]))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.trim().to_string())
}

/// Get local IPv4 address on the specified interface (works for WiFi or wired)
pub fn get_local_ip(interface: &WiFiInterface) -> Result<std::net::Ipv4Addr, FCError> {
    let cidr = get_ip_cidr(interface)?;
    let ip_str = cidr.split('/').next().unwrap_or("");
    ip_str.parse().map_err(|e| FCError {
        message: format!("Failed to parse IP address '{}': {}", ip_str, e),
    })
}

/// Get the subnet prefix length (e.g. 24 for /24) on the specified interface
pub fn get_prefix_length(interface: &WiFiInterface) -> Result<u8, FCError> {
    let cidr = get_ip_cidr(interface)?;
    let prefix_str = cidr.split('/').nth(1).unwrap_or("24");
    prefix_str.parse().map_err(|e| FCError {
        message: format!("Failed to parse prefix length '{}': {}", prefix_str, e),
    })
}

/// Returns the CIDR notation (e.g. "192.168.1.100/24") for the interface
fn get_ip_cidr(interface: &WiFiInterface) -> Result<String, FCError> {
    let ip_command = format!(
        "ip -4 addr show {} | grep inet | awk '{{print $2}}'",
        interface.0
    );
    let output = run_command("sh", Some(vec!["-c", &ip_command]))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let cidr = stdout.trim();

    if cidr.is_empty() {
        fc_error(&format!(
            "No IPv4 address found on interface {}",
            interface.0
        ))?;
    }

    Ok(cidr.to_string())
}

/// No-op on Linux: firewall rules are only managed on Windows.
pub async fn ensure_firewall_rules<T: UI>(_ui: &T) -> Result<(), FCError> {
    Ok(())
}

/// Check if interface has an active network connection
pub fn has_network_connection(interface: &WiFiInterface) -> Result<bool, FCError> {
    // Check if interface has an IP address assigned
    let ip_command = format!("ip -4 addr show {} | grep inet", interface.0);
    let output = run_command("sh", Some(vec!["-c", &ip_command]))?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    Ok(!stdout.trim().is_empty())
}

/// Get WiFi and Ethernet interfaces that have an IPv4 address, for shared network
/// mode (which works over wired connections too, unlike hotspot mode). Filtering by
/// nmcli device type keeps virtual interfaces (docker0, VPN tunnels, bridges) out of
/// the interface chooser.
pub fn get_connected_interfaces() -> Result<Vec<InterfaceInfo>, FCError> {
    let command_output = run_command("nmcli", Some(vec!["-t", "device"]))?;
    let output = String::from_utf8_lossy(&command_output.stdout);
    let default_iface = default_route_interface();
    let mut with_gateway = Vec::new();
    let mut without_gateway = Vec::new();
    for line in output.lines() {
        // Format: DEVICE:TYPE:STATE:CONNECTION
        let split_line: Vec<&str> = line.split(':').collect();
        if split_line.len() < 2 {
            continue;
        }
        if split_line[1] != "wifi" && split_line[1] != "ethernet" {
            continue;
        }
        let name = split_line[0].to_string();
        // omit interfaces without a usable IPv4: they can't work in shared mode
        let ip = match interface_ipv4(&name) {
            Some(ip) => ip,
            None => continue,
        };
        let is_default = default_iface.as_deref() == Some(name.as_str());
        let info = InterfaceInfo {
            name,
            guid: String::new(),
            ip: Some(ip),
        };
        // list the default-route interface first so the UI can preselect it
        if is_default {
            with_gateway.push(info);
        } else {
            without_gateway.push(info);
        }
    }
    with_gateway.append(&mut without_gateway);
    Ok(with_gateway)
}

fn get_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string())
}

#[cfg(test)]
mod test {
    use crate::{PeerResource, UI};

    use super::get_wifi_interfaces;

    #[test]
    fn start_and_stop_hotspot() {
        let ssid = "flyingCarpet_1234";
        let password = "password";
        let _pr = PeerResource::WifiClient("".to_string());
        let interface = &get_wifi_interfaces().expect("no wifi interface present")[0].name;
        crate::network::start_hotspot(ssid, password, interface).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(5));
        crate::network::stop_hotspot(Some(&_pr), Some(ssid)).unwrap();
    }

    #[test]
    fn join_hotspot() {
        #[derive(Clone)]
        struct TestUI {}
        impl UI for TestUI {
            fn output(&self, _msg: &str) {}
            fn show_progress_bar(&self) {}
            fn update_progress_bar(&self, _percent: u8) {}
            fn enable_ui(&self) {}
            fn show_pin(&self, _pin: &str) {}
        }

        let ssid = "";
        let password = "";
        let pr = PeerResource::WifiClient("".to_string());
        let interface = &get_wifi_interfaces().expect("no wifi interface present")[0].name;
        let interface = interface.to_string();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
        tokio::spawn(async move {
            crate::network::join_hotspot(ssid, password, &interface, &TestUI {})
                .await
                .unwrap();
            std::thread::sleep(std::time::Duration::from_secs(20));
            crate::network::stop_hotspot(Some(&pr), Some(ssid)).unwrap();
            tx.send(()).await.unwrap();
        });
        rx.blocking_recv().unwrap();
    }

    #[test]
    fn find_gateway() {
        let interface = &get_wifi_interfaces().expect("no wifi interface present")[0].name;
        let gateway = crate::network::find_gateway(interface).unwrap();
        println!("interface: {}", interface);
        println!("gateway: {}", gateway);
    }
}
