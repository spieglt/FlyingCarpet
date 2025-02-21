mod central;
mod peripheral;

use bluer::{Adapter, Session};
use central::{exchange_info, find_characteristics};
use std::{error::Error, mem::discriminant, time::Duration};
use tokio::{sync::mpsc, time::sleep};

use crate::{
    network::is_hosting,
    utils::{generate_password, get_key_and_ssid, BluetoothMessage},
    Mode, Peer, UI,
};

pub(crate) const OS: &str = "linux";
const SERVICE_UUID: &str = "A70BF3CA-F708-4314-8A0E-5E37C259BE5C";
pub(crate) const OS_CHARACTERISTIC_UUID: &str = "BEE14848-CC55-4FDE-8E9D-2E0F9EC45946";
pub(crate) const SSID_CHARACTERISTIC_UUID: &str = "0D820768-A329-4ED4-8F53-BDF364EDAC75";
pub(crate) const PASSWORD_CHARACTERISTIC_UUID: &str = "E1FA8F66-CF88-4572-9527-D5125A2E0762";
// const NO_SSID: &str = "NONE";

pub async fn check_support() -> Result<(), Box<dyn Error>> {
    let session = Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;
    println!("Bluetooth is supported");
    Ok(())
}

pub async fn get_adapter() -> Result<Adapter, Box<dyn Error>> {
    let session = Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;
    println!("Bluetooth is supported");
    Ok(adapter)
}

pub async fn negotiate_bluetooth<T: UI>(
    mode: &Mode,
    _ble_ui_rx: mpsc::Receiver<bool>, // only used on windows
    ui: &T,
) -> Result<(String, String, String), Box<dyn Error>> {
    // TODO: dedup with check_support(), but can't return adapter from it because windows doesn't, unless we stub which is annoying to pass it back into this.
    let session = Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;

    if let Mode::Send(_) = mode {
        // acting as peripheral
        let (tx, mut rx) = mpsc::channel(1);
        let mut password = generate_password();
        let (_, mut ssid) = get_key_and_ssid(&password);
        let (app_handle, adv_handle) = peripheral::advertise(tx, &ssid, &password).await?;
        let peer_os =
            match process_bluetooth_message(BluetoothMessage::PeerOS("".to_string()), &mut rx, ui)
                .await?
            {
                BluetoothMessage::PeerOS(os) => os,
                other => Err(format!(
                    "Received unexpected BluetoothMessage when waiting for peer OS: {:?}",
                    other
                ))?,
            };

        println!("Removing advertisement");
        drop(adv_handle);

        if is_hosting(&Peer::from(peer_os.as_str()), mode) {
            // wait for peer to read our ssid and password
            process_bluetooth_message(BluetoothMessage::PeerReadSsid, &mut rx, ui).await?;
            println!("Peer read SSID");
            process_bluetooth_message(BluetoothMessage::PeerReadPassword, &mut rx, ui).await?;
            println!("Peer read password");
        } else {
            // wait for peer to write its ssid and password
            ssid = match process_bluetooth_message(
                BluetoothMessage::SSID("".to_string()),
                &mut rx,
                ui,
            )
            .await?
            {
                BluetoothMessage::SSID(s) => s,
                other => Err(format!(
                    "Received unexpected BluetoothMessage when waiting for peer OS: {:?}",
                    other
                ))?,
            };
            println!("Peer's SSID: {}", ssid);
            password = match process_bluetooth_message(
                BluetoothMessage::Password("".to_string()),
                &mut rx,
                ui,
            )
            .await?
            {
                BluetoothMessage::Password(p) => p,
                other => Err(format!(
                    "Received unexpected BluetoothMessage when waiting for peer OS: {:?}",
                    other
                ))?,
            };
            println!("Peer's password: {}", password);
        }

        sleep(Duration::from_secs(1)).await;
        println!("Removing GATT service");
        drop(app_handle);

        Ok((peer_os, ssid, password))
    } else {
        // acting as central
        ui.output("Scanning for Bluetooth peripherals...");
        // let info = central::scan(&adapter, &mode).await?;
        let device = central::scan(&adapter).await?;

        let characteristics = match find_characteristics(&device).await {
            Ok(c) => c,
            Err(e) => {
                println!("    Device failed: {}", e);
                let _ = adapter.remove_device(device.address()).await;
                Err(e)?
            }
        };
        let info = match exchange_info(characteristics, mode).await {
            Ok(i) => i,
            Err(e) => {
                let _ = adapter.remove_device(device.address()).await;
                Err(e)?
            }
        };
        Ok(info)
    }
}

// TODO: make linux-appropriate
pub async fn process_bluetooth_message<T: UI>(
    looking_for: BluetoothMessage,
    rx: &mut mpsc::Receiver<BluetoothMessage>,
    ui: &T,
) -> Result<BluetoothMessage, Box<dyn Error>> {
    loop {
        println!("waiting for bluetooth message...");
        let msg = rx
            .recv()
            .await
            .expect("Bluetooth message channel unexpectedly closed.");
        println!("received {:?}", msg);
        match &msg {
            BluetoothMessage::PairApproved => ui.output("Pairing approved."),
            BluetoothMessage::PairSuccess => {
                // can use this to represent AlreadyPaired on windows? don't need to emit pin, just need to proceed.
                // and nothing will be blocked in central because the pairing_handler won't be called.
                ui.output("Successfully paired");
            }
            BluetoothMessage::PairFailure => Err("Pairing failed.")?,
            BluetoothMessage::AlreadyPaired => {
                ui.output("Already BLE paired with Bluetooth device");
                if looking_for == BluetoothMessage::PairSuccess {
                    return Ok(msg);
                }
            }
            BluetoothMessage::UserCanceled => Err("User canceled.")?,
            BluetoothMessage::StartedAdvertising => {
                ui.output("Started advertising Bluetooth service")
            }
            BluetoothMessage::PeerOS(os) => ui.output(&format!("Peer's OS is {}", os)),
            BluetoothMessage::SSID(ssid) => ui.output(&format!("Peer's SSID is {}", ssid)),
            BluetoothMessage::Password(password) => {
                ui.output(&format!("Peer's password is {}", password))
            }
            BluetoothMessage::PeerReadSsid => ui.output("Peer read our SSID"),
            BluetoothMessage::PeerReadPassword => ui.output("Peer read our password"),
            BluetoothMessage::OtherError(s) => Err(s.as_str())?, // ui.output(&format!("Bluetooth peering result: {}", s)),
            other_message => println!(
                "Other Bluetooth message not used on Linux: {:?}",
                other_message
            ),
        };
        if discriminant(&msg) == discriminant(&looking_for) {
            return Ok(msg);
        }
    }
}
