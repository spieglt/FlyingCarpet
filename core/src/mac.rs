use regex::Regex;

use crate::{utils, PeerResource};

use super::{Mode, Peer, UI};
use std::error::Error;
use std::ffi::c_char;
use std::process;

extern "C" {
    fn joinAdHoc(cSSID: *const c_char, cPassword: *const c_char) -> u8;
}

// unused stub because this is referenced in lib.rs
pub struct WindowsHotspot {
    _inner: (),
}

pub fn stop_hotspot(peer_resource: &PeerResource) -> Result<(), Box<dyn Error>> {
    if let PeerResource::WifiClient(_gateway, ssid) = peer_resource {
        let interface = get_wifi_interface();
        let output = process::Command::new("networksetup")
            .args(vec!["-removepreferredwirelessnetwork", &interface, ssid])
            .output()?;
        let _stdout = String::from_utf8_lossy(&output.stdout);
        // println!("{}", _stdout);

        for mode in ["off", "on"] {
            process::Command::new("networksetup")
                .args(vec!["-setairportpower", &interface, mode])
                .output()?;
        }
    }
    Ok(())
}

pub async fn connect_to_peer<T: UI>(
    _peer: Peer,
    _mode: Mode,
    ssid: String,
    password: String,
    ui: &T,
) -> Result<PeerResource, Box<dyn Error>> {
    // mac never hosts
    loop {
        ui.output(&format!("Trying to join hotspot {}...", ssid));
        unsafe {
            match join_hotspot(&ssid, &password) {
                Ok(()) => {
                    // println!("Connected to {}", ssid);
                    break;
                }
                Err(_e) => {
                    // println!("Error: {}", _e);
                }
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
    loop {
        // println!("looking for gateway");
        if let Some(gateway) = find_gateway() {
            return Ok(PeerResource::WifiClient(gateway, ssid));
        } else {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }
    }
}

unsafe fn join_hotspot(ssid: &str, password: &str) -> Result<(), Box<dyn Error>> {
    let ssid = utils::rust_to_c_string(ssid);
    let password = utils::rust_to_c_string(&password);
    if joinAdHoc(ssid, password) == 0 {
        Err("couldn't find hotspot")?;
    }
    Ok(())
}

fn find_gateway() -> Option<String> {
    let interface = get_wifi_interface();
    if interface == "" {
        return None;
    }
    let output = process::Command::new("ipconfig")
        .args(vec!["getsummary", &interface])
        .output()
        .expect("Couldn't run ipconfig");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let pattern =
        Regex::new(r"Router *: *(?P<ip>[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3})").unwrap();
    pattern
        .captures(&stdout)
        .map(|caps| caps.name("ip").unwrap().as_str().to_string()) // unwrap ok because any captures are guaranteed to have an ip group?
}

fn get_wifi_interface() -> String {
    let args = vec![
        "-c",
        "networksetup -listallhardwareports | awk '/Wi-Fi/{getline; print $2}'",
    ];
    let output = process::Command::new("sh")
        .args(args)
        .output()
        .expect("Couldn't get WiFi interface");
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.trim().to_string()
}

#[cfg(test)]
mod test {
    use crate::PeerResource;

    #[test]
    fn get_wifi_interface() {
        let interface = crate::network::get_wifi_interface();
        println!("wifi interface: {}", interface);
    }

    #[test]
    fn delete_network() {
        match super::stop_hotspot(&PeerResource::WifiClient("".to_string(), "".to_string())) {
            Ok(()) => (),
            Err(e) => println!("{:?}", e),
        };
    }

    #[test]
    fn join_network() {
        unsafe {
            match super::join_hotspot("", "") {
                Ok(()) => (),
                Err(e) => println!("{:?}", e),
            };
        }
    }
}
