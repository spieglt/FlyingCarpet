mod central;
mod peripheral;

use bluer::{Adapter, DiscoveryFilter, DiscoveryTransport, Session, Uuid};
use std::{collections::HashSet, error::Error};
use tokio::sync::mpsc;

use crate::{utils::BluetoothMessage, Mode, UI};
// use central::BluetoothCentral;
use peripheral::BluetoothPeripheral;

pub(crate) const OS: &str = "linux";
const SERVICE_UUID: &str = "A70BF3CA-F708-4314-8A0E-5E37C259BE5C";
pub(crate) const OS_CHARACTERISTIC_UUID: &str = "BEE14848-CC55-4FDE-8E9D-2E0F9EC45946";
pub(crate) const SSID_CHARACTERISTIC_UUID: &str = "0D820768-A329-4ED4-8F53-BDF364EDAC75";
pub(crate) const PASSWORD_CHARACTERISTIC_UUID: &str = "E1FA8F66-CF88-4572-9527-D5125A2E0762";
const NO_SSID: &str = "NONE";

pub(crate) struct Bluetooth {
    // pub central: BluetoothCentral,
    pub peripheral: BluetoothPeripheral,
    adapter: Adapter,
}

pub async fn check_support() -> Result<(), Box<dyn Error>> {
    let session = Session::new().await?;
    let adapter = session.default_adapter().await?;
    println!(
        "Discovering devices using Bluetooth adapter {}\n",
        adapter.name()
    );
    adapter.set_powered(true).await?;

    Ok(())
}

pub async fn negotiate_bluetooth<T: UI>(
    mode: &Mode,
    ble_ui_rx: mpsc::Receiver<bool>,
    ui: &T,
) -> Result<(String, String, String), Box<dyn Error>> {
    let peer = String::new();
    let ssid = String::new();
    let password = String::new();
    Ok((peer, ssid, password))
}
