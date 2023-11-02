use regex::Regex;

use crate::{utils, PeerResource, WiFiInterface};

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

pub fn stop_hotspot(_peer_resource: Option<&PeerResource>, ssid: Option<&str>) -> Result<(), Box<dyn Error>> {
    if let Some(ssid) = ssid {
        let interface = get_wifi_interfaces()?[0].0.to_string();
        let output = process::Command::new("networksetup")
            .args(vec!["-removepreferredwirelessnetwork", &interface, ssid])
            .output()?;
        let _stdout = String::from_utf8_lossy(&output.stdout);

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
    interface: WiFiInterface,
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
        if let Some(gateway) = find_gateway(&interface) {
            return Ok(PeerResource::WifiClient(gateway));
        } else {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }
    }
}

unsafe fn join_hotspot(ssid: &str, password: &str) -> Result<(), Box<dyn Error>> {
    // TODO: these are never reclaimed
    let ssid = utils::rust_to_c_string(ssid);
    let password = utils::rust_to_c_string(&password);
    if joinAdHoc(ssid, password) == 0 {
        Err("couldn't find hotspot")?;
    }
    Ok(())
}

fn find_gateway(interface: &WiFiInterface) -> Option<String> {
    if interface.0 == "" {
        return None;
    }
    let output = process::Command::new("ipconfig")
        .args(vec!["getsummary", &interface.0])
        .output()
        .expect("Couldn't run ipconfig");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let pattern =
        Regex::new(r"Router *: *(?P<ip>[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3})").unwrap();
    pattern
        .captures(&stdout)
        .map(|caps| caps.name("ip").unwrap().as_str().to_string()) // unwrap ok because any captures are guaranteed to have an ip group?
}

// unlike linux and windows, this only returns the built-in WiFi.
// using a second wireless card on macOS is not straightforward or common,
// nor would it be easy to scan for here, so calling this sufficient.
pub fn get_wifi_interfaces() -> Result<Vec<WiFiInterface>, Box<dyn Error>> {
    let args = vec![
        "-c",
        "networksetup -listallhardwareports | awk '/Wi-Fi/{getline; print $2}'",
    ];
    let output = process::Command::new("sh").args(args).output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(vec![WiFiInterface(
        stdout.trim().to_string(),
        "".to_string(),
    )])
}

#[cfg(test)]
mod test {

    #[test]
    fn get_wifi_interface() {
        let interface =
            &crate::network::get_wifi_interfaces().expect("no wifi interface present")[0];
        println!("wifi interface: {}", interface.0);
    }

    #[test]
    fn delete_network() {
        match super::stop_hotspot(None, Some("flyingCarpet_abcd")) {
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
