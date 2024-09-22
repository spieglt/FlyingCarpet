mod central;
mod peripheral;

use std::{error::Error, sync::mpsc::Sender};

use central::BluetoothCentral;
use peripheral::BluetoothPeripheral;
use windows::{
    core::GUID,
    Storage::Streams::{DataReader, UnicodeEncoding},
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
    if !central::check_support()? {
        Err("Central role not supported")?;
    }
    println!("Central role is supported");
    if !peripheral::check_support()? {
        Err("Peripheral role not supported")?;
    }
    println!("Peripheral role is supported");
    Ok(())
}

impl Bluetooth {
    pub fn new() -> Result<Self, String> { // returning Result<Self, Box<dyn Error>> here was throwing weird tokio errors so punting to string
        let peripheral_support = peripheral::check_support()
            .map_err(|e| format!("Error checking for peripheral support: {}", e))?;
        if !peripheral_support {
            Err("Device does not support acting as a Bluetooth LE peripheral")?;
        }
        let peripheral = BluetoothPeripheral::new().map_err(|e| e.to_string())?;

        let central_support = central::check_support()
            .map_err(|e| format!("Error checking for central support: {}", e))?;
        if !central_support {
            Err("Device does not support acting as a Bluetooth LE central.")?;
        }
        let central = BluetoothCentral::new().map_err(|e| e.to_string())?;

        Ok(Bluetooth {
            peripheral,
            central,
        })
    }

    async fn initialize_bluetooth(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.central = BluetoothCentral::new()?;
        self.peripheral = BluetoothPeripheral::new()?;

        // stop watching for advertisements
        // _watcher.Stop()?;


        // bluetooth_peripheral.add_characteristic()?;
        // bluetooth_peripheral.start_advertising()?;

        Ok(())
    }
}

async fn _connect_to_device(_address: u64) -> Result<(), Box<dyn Error>> {
    Ok(())
}

// https://stackoverflow.com/a/38704180/9242143
