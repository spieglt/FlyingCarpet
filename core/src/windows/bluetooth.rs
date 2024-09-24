mod central;
mod peripheral;

use std::error::Error;

use central::BluetoothCentral;
use peripheral::BluetoothPeripheral;
use windows::{
    core::GUID, Devices::{Bluetooth::BluetoothAdapter, Radios::RadioState},
};

const SERVICE_UUID: &str = "A70BF3CA-F708-4314-8A0E-5E37C259BE5C";
const OS_CHARACTERISTIC_UUID: &str = "BEE14848-CC55-4FDE-8E9D-2E0F9EC45946";
const SSID_CHARACTERISTIC_UUID: &str = "0D820768-A329-4ED4-8F53-BDF364EDAC75";
const PASSWORD_CHARACTERISTIC_UUID: &str = "E1FA8F66-CF88-4572-9527-D5125A2E0762";
const NO_SSID: &str = "NONE";

pub(crate) struct Bluetooth {
    central: BluetoothCentral,
    peripheral: BluetoothPeripheral,
}

// central goes scan -> bond -> connect -> discoverServices -> read OS -> write OS
// -> connectToPeer -> start hotspot and write ssid/pw, or read ssid/pw and join hotspot

// peripheral goes advertise, wait for bonding, wait for OS read, wait for OS write,
// connectToPeer, start hotspot and wait for ssid/password to be read, or wait for ssid/pw writes and joinHotspot

pub fn check_support() -> Result<(), Box<dyn Error>> {
    let adapter = BluetoothAdapter::GetDefaultAsync()?.get()?;
    let radio = adapter.GetRadioAsync()?.get()?;
    if radio.State()? != RadioState::On {
        Err("radio is not on")?;
    }
    if !adapter.IsCentralRoleSupported()? {
        Err("central role not supported")?;
    }
    println!("Central role is supported");
    if !adapter.IsPeripheralRoleSupported()? {
        Err("peripheral role not supported")?;
    }
    println!("Peripheral role is supported");
    Ok(())
}

impl Bluetooth {
    pub fn new() -> Result<Self, String> { // returning Result<Self, Box<dyn Error>> here was throwing weird tokio errors so punting to string
        let peripheral = BluetoothPeripheral::new().map_err(|e| e.to_string())?;
        let central = BluetoothCentral::new().map_err(|e| e.to_string())?;

        Ok(Bluetooth {
            peripheral,
            central,
        })
    }
}

async fn _connect_to_device(_address: u64) -> Result<(), Box<dyn Error>> {
    Ok(())
}

// https://stackoverflow.com/a/38704180/9242143
