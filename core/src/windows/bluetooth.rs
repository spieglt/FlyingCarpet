mod central;
mod peripheral;

use std::error::Error;

use central::BluetoothCentral;
use peripheral::BluetoothPeripheral;
use tokio::sync::mpsc;
use windows::{
    core::HSTRING,
    Devices::{Bluetooth::BluetoothAdapter, Radios::RadioState},
    Storage::Streams::{DataReader, DataWriter, IBuffer, UnicodeEncoding},
};

pub(crate) const OS: &str = "windows";
const SERVICE_UUID: &str = "A70BF3CA-F708-4314-8A0E-5E37C259BE5C";
pub(crate) const OS_CHARACTERISTIC_UUID: &str = "BEE14848-CC55-4FDE-8E9D-2E0F9EC45946";
pub(crate) const SSID_CHARACTERISTIC_UUID: &str = "0D820768-A329-4ED4-8F53-BDF364EDAC75";
pub(crate) const PASSWORD_CHARACTERISTIC_UUID: &str = "E1FA8F66-CF88-4572-9527-D5125A2E0762";
// android uses "NONE" to say "the hotspot isn't up yet, so we don't know the SSID yet" because it's given by the android OS
// do we need this on windows/linux? if we're hosting, we know the SSID because we generate the password.
// do we need to delay reporting the OS until the hotspot is stood up? no, not necessarily.
// but do we need this for communicating with android? not necessarily, because windows and linux will both host if communicating with android.
// however, it might be good to future-proof and allow for this codebase to understand that signal from android,
// in case hosting rules change, which would mean detecting this when reading ssid and delaying/retrying.
const NO_SSID: &str = "NONE";

// can just match and only look for the type of message we want each read,
// and only need one rx channel?
#[derive(Debug, PartialEq)]
pub enum BluetoothMessage {
    Pin(String),
    PairSuccess,
    PairFailure,
    AlreadyPaired,
    UserCanceled,
    StartedAdvertising,
    PeerOS(String),
    SSID(String),
    Password(String),
    PeerReadSsid,
    PeerReadPassword,
    Other(String),
}

unsafe impl Send for BluetoothMessage {}
unsafe impl Sync for BluetoothMessage {}

pub(crate) struct Bluetooth {
    pub central: BluetoothCentral,
    pub peripheral: BluetoothPeripheral,
}

// central goes scan -> bond -> connect -> discoverServices -> read OS -> write OS
// -> connectToPeer -> start hotspot and write ssid/pw, or read ssid/pw and join hotspot

// peripheral goes advertise, wait for bonding, wait for OS read, wait for OS write,
// connectToPeer, start hotspot and wait for ssid/password to be read, or wait for ssid/pw writes and joinHotspot

pub fn check_support() -> Result<(), Box<dyn Error>> {
    let adapter = BluetoothAdapter::GetDefaultAsync()?
        .get()
        .map_err(|_| "no adapter found")?;
    println!("got adapter");
    let radio = adapter
        .GetRadioAsync()?
        .get()
        .map_err(|_| "could not find radio")?;
    println!("got radio");
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
    pub fn new(tx: mpsc::Sender<BluetoothMessage>) -> Result<Self, String> {
        // returning Result<Self, Box<dyn Error>> here was throwing weird tokio errors so punting to string
        let peripheral = BluetoothPeripheral::new(tx.clone()).map_err(|e| e.to_string())?;
        let central = BluetoothCentral::new(tx.clone()).map_err(|e| e.to_string())?;

        Ok(Bluetooth {
            peripheral,
            central,
        })
    }
}

async fn _connect_to_device(_address: u64) -> Result<(), Box<dyn Error>> {
    Ok(())
}

fn ibuffer_to_string(ibuffer: IBuffer) -> windows::core::Result<String> {
    let size = ibuffer.Capacity()?;
    let data_reader = DataReader::FromBuffer(&ibuffer)?;
    data_reader.SetUnicodeEncoding(UnicodeEncoding::Utf8)?;
    Ok(data_reader.ReadString(size)?.to_string())
}

fn str_to_ibuffer(s: &str) -> windows::core::Result<IBuffer> {
    let data_writer = DataWriter::new()?;
    let bytes_written = data_writer.WriteString(&HSTRING::from(s))?; // TODO: is this utf-8? WriteBytes instead?
    println!("bytes written: {}", bytes_written);
    Ok(data_writer.DetachBuffer()?)
}

// https://stackoverflow.com/a/38704180/9242143
