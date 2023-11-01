use crate::utils::run_command;
use crate::{Mode, Peer, PeerResource, WiFiInterface, UI};
use std::error::Error;
use tokio::task;

// stub
pub struct WindowsHotspot {
    _inner: (),
}

fn is_hosting(peer: Peer, mode: Mode) -> bool {
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
) -> Result<PeerResource, Box<dyn Error>> {
    if is_hosting(peer, mode) {
        // start hotspot
        ui.output(&format!("Starting hotspot {}", ssid));
        start_hotspot(&ssid, &password, &interface.0)?;
        Ok(PeerResource::LinuxHotspot)
    } else {
        // join hotspot and find gateway
        ui.output(&format!("Joining hotspot {}", ssid));
        join_hotspot(&ssid, &password, &interface.0)?;
        loop {
            // println!("looking for gateway");
            task::yield_now().await;
            match find_gateway(&interface.0) {
                Ok(gateway) => {
                    if gateway != "" {
                        return Ok(PeerResource::WifiClient(gateway, ssid));
                    }
                }
                Err(e) => Err(e)?,
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }
    }
}

fn start_hotspot(ssid: &str, password: &str, interface: &str) -> Result<(), Box<dyn Error>> {
    let nmcli = "nmcli";
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
            Err(format!("Could not start hotspot: {}", stderr))?;
        }
        // println!("output: {}", String::from_utf8_lossy(&res.stdout));
    }
    Ok(())
}

pub fn stop_hotspot(peer_resource: &PeerResource) -> Result<(), Box<dyn Error>> {
    match peer_resource {
        PeerResource::LinuxHotspot(ssid) | PeerResource::WifiClient(_, ssid) => {
            let options = Some(vec!["connection", "delete", ssid]);
            let command_output = run_command("nmcli", options)?;
            if !command_output.status.success() {
                let stderr = String::from_utf8_lossy(&command_output.stderr);
                Err(format!("Error stopping hotspot: {}", stderr))?;
            }
            // let output = String::from_utf8_lossy(&command_output.stdout);
            // println!("Stop hotspot output: {}", output);
        }
        _ => (),
    }
    Ok(())
}

fn join_hotspot(ssid: &str, password: &str, interface: &str) -> Result<(), Box<dyn Error>> {
    let nmcli = "nmcli";
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
        ],
        vec!["con", "modify", ssid, "wifi-sec.key-mgmt", "wpa-psk"],
        vec!["con", "modify", ssid, "wifi-sec.psk", password],
        vec!["con", "up", ssid],
    ];
    for command in commands {
        let res = run_command(nmcli, Some(command))?;
        if !res.status.success() {
            let stderr = String::from_utf8_lossy(&res.stderr);
            Err(format!("Error joining hotspot: {}", stderr))?;
        }
        // println!(
        //     "join hotspot output: {}",
        //     String::from_utf8_lossy(&res.stdout)
        // );
    }
    Ok(())
}

pub fn get_wifi_interfaces() -> Result<Vec<WiFiInterface>, Box<dyn Error>> {
    let command = "nmcli";
    let options = vec!["-t", "device"];
    let command_output = run_command(command, Some(options))?;
    let output = String::from_utf8_lossy(&command_output.stdout);
    let mut interfaces: Vec<WiFiInterface> = vec![];
    output
        .lines()
        .map(|line| line.split(":").collect())
        .for_each(|split_line: Vec<&str>| {
            if split_line[1] == "wifi" {
                interfaces.push(WiFiInterface(split_line[0].to_string(), "".to_string()));
            }
        });
    Ok(interfaces)
}

fn find_gateway(interface: &str) -> Result<String, Box<dyn Error>> {
    let route_command = format!(
        "route -n | grep {} | grep UG | awk '{{print $2}}'",
        interface
    ); // TODO: not the best but it will do? use regex in rust?
    let output = run_command("sh", Some(vec!["-c", &route_command]))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.trim().to_string())
}

#[cfg(test)]
mod test {
    use crate::PeerResource;

    use super::get_wifi_interfaces;

    #[test]
    fn start_and_stop_hotspot() {
        let ssid = "flyingCarpet_1234";
        let password = "password";
        let _pr = PeerResource::WifiClient("".to_string(), ssid.to_string());
        let interface = &get_wifi_interfaces().expect("no wifi interface present")[0].0;
        crate::network::start_hotspot(ssid, password, interface).unwrap();
        // std::thread::sleep(std::time::Duration::from_secs(500));
        // crate::network::stop_hotspot(&pr).unwrap();
    }

    #[test]
    fn join_hotspot() {
        let ssid = "";
        let password = "";
        let pr = PeerResource::WifiClient("".to_string(), ssid.to_string());
        let interface = &get_wifi_interfaces().expect("no wifi interface present")[0].0;
        crate::network::join_hotspot(ssid, password, &interface).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(20));
        crate::network::stop_hotspot(&pr).unwrap();
    }

    #[test]
    fn find_gateway() {
        let interface = &get_wifi_interfaces().expect("no wifi interface present")[0].0;
        let gateway = crate::network::find_gateway(interface).unwrap();
        println!("interface: {}", interface);
        println!("gateway: {}", gateway);
    }
}
