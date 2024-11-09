mod central;
mod peripheral;

use std::{error::Error, mem::discriminant};

use crate::{
    network::{self, is_hosting},
    utils::{generate_password, get_key_and_ssid},
    Mode, Peer, UI,
};
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

pub async fn negotiate_bluetooth<T: UI>(
    mode: &Mode,
    ble_ui_rx: mpsc::Receiver<bool>,
    ui: &T,
) -> Result<(String, String, String), Box<dyn Error>> {
    let (tx, mut rx) = mpsc::channel(1);
    let mut bluetooth = Bluetooth::new(tx)?;
    if let Mode::Send(_) = mode {
        ui.output("Advertising Bluetooth service...");
        bluetooth.peripheral.add_characteristics()?;
        bluetooth.peripheral.start_advertising()?;

        let mut peer_os = String::new();
        let mut peer_ssid = String::new();
        let mut peer_password = String::new();

        // ensure we started advertising
        process_bluetooth_message(BluetoothMessage::StartedAdvertising, &mut rx, ui).await?;

        // get OS of peer
        let msg =
            process_bluetooth_message(BluetoothMessage::PeerOS(String::new()), &mut rx, ui).await?;
        if let BluetoothMessage::PeerOS(os) = msg {
            peer_os = os;
        } else {
            Err(format!(
                "Peripheral received incorrect BluetoothMessage. Expected peer OS, got {:?}",
                msg
            ))?;
        }

        if is_hosting(&Peer::from(peer_os.as_str()), mode) {
            let password = generate_password();
            // TODO: race condition here, if peer reads from our SSID characteristic before we've set it?
            // then we'll write NONE, peer will wait a second and read again, so tx will get another PeerReadSSID message,
            // making the "waiting for password" BluetoothMessage panic? only send PeerReadSSID if we sent a real one?
            // or pass the info in earlier so we're guaranteed to have it? but can we do this safely before we've exchanged
            // OS and know if we're hosting? doesn't hurt to have the data set even if we're not hosting maybe, but it's ugly.
            let (_, ssid) = get_key_and_ssid(&password);
            {
                let mut peripheral_ssid = bluetooth.peripheral.ssid.lock().await;
                *peripheral_ssid = Some(ssid.clone());
                let mut peripheral_password = bluetooth.peripheral.password.lock().await;
                *peripheral_password = Some(password.clone());
            }
            println!("set peripheral ssid and password");
            println!("waiting for ssid to be read...");
            process_bluetooth_message(BluetoothMessage::PeerReadSsid, &mut rx, ui).await?;
            println!("waiting for password to be read...");
            process_bluetooth_message(BluetoothMessage::PeerReadPassword, &mut rx, ui).await?;
            Ok((peer_os, ssid.clone(), password))
        } else {
            // if joining, receive writes
            // receive ssid
            let msg = process_bluetooth_message(BluetoothMessage::SSID(String::new()), &mut rx, ui)
                .await?;
            if let BluetoothMessage::SSID(ssid) = msg {
                peer_ssid = ssid;
            } else {
                Err(format!(
                    "Peripheral received incorrect BluetoothMessage. Expected SSID, got {:?}",
                    msg
                ))?;
            }
            // receive password
            let msg =
                process_bluetooth_message(BluetoothMessage::Password(String::new()), &mut rx, ui)
                    .await?;
            if let BluetoothMessage::Password(password) = msg {
                peer_password = password;
            } else {
                Err(format!(
                    "Peripheral received incorrect BluetoothMessage. Expected password, got {:?}",
                    msg
                ))?;
            }
            Ok((peer_os, peer_ssid, peer_password))
        }
    } else {
        // scan for device advertising flying carpet service
        ui.output("Scanning for Bluetooth peripherals...");
        bluetooth.central.scan(ble_ui_rx)?;

        // wait for result of scan. if PIN was shown, wait again for success or failure.
        // TODO: don't need to wait for PIN, just result of scan? we don't do anything with the PIN here. can just have process_bluetooth_message() print that we received it.
        // no, do need to wait for the result of the pin so we don't move on till user has made their choice.
        // how to handle this on linux? just send a dummy?
        // problem: we don't need to just wait for the user to hit yes on the pin dialog. we need to wait till we actually pair.
        // if we hit yes but peer doesn't, we'll try to read characteristics that are still encrypted.
        println!("waiting for callback...");
        let msg = process_bluetooth_message(BluetoothMessage::Pin("".to_string()), &mut rx, ui).await?;

        bluetooth.central.stop_watching()?;
        println!("stopped watching");

        // TODO: do we need to wait to be notified that we've paired here, or just wait till central reads OS? don't think we get notification that central has paired with us in linux.
        // but if we don't, how will we know if user hit cancel on pairing dialog?
        // wait to pair
        if msg != BluetoothMessage::AlreadyPaired {
            process_bluetooth_message(BluetoothMessage::PairSuccess, &mut rx, ui).await?;
        }

        println!("before get_services_and_characteristics");
        // discover service and characteristics once paired
        bluetooth.central.get_services_and_characteristics().await?;
        println!("after get_services_and_characteristics");

        ui.output("Reading peer's OS");
        // read peer's OS
        let peer = bluetooth.central.read(OS_CHARACTERISTIC_UUID).await?;
        ui.output(&format!("Peer OS: {:?}", peer));

        // write OS
        bluetooth.central.write(OS_CHARACTERISTIC_UUID, OS).await?;

        // read or write ssid and password
        let (ssid, password) = if network::is_hosting(&Peer::from(peer.as_str()), mode) {
            let password = generate_password();
            let (_, ssid) = get_key_and_ssid(&password);
            bluetooth
                .central
                .write(SSID_CHARACTERISTIC_UUID, &ssid)
                .await?;
            bluetooth
                .central
                .write(PASSWORD_CHARACTERISTIC_UUID, &password)
                .await?;
            (ssid, password)
        } else {
            let ssid = bluetooth.central.read(SSID_CHARACTERISTIC_UUID).await?;
            let password = bluetooth.central.read(PASSWORD_CHARACTERISTIC_UUID).await?;
            (ssid, password)
        };
        Ok((peer, ssid, password))
    }
}

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
        match msg {
            BluetoothMessage::Pin(ref pin) => {
                ui.show_pin(pin);
            }
            BluetoothMessage::PairApproved => ui.output("Pairing approved."),
            BluetoothMessage::PairSuccess => {
                // can use this to represent AlreadyPaired on windows? don't need to emit pin, just need to proceed.
                // and nothing will be blocked in central because the pairing_handler won't be called.
                ui.output("Successfully paired");
            }
            BluetoothMessage::PairFailure => Err("Pairing failed.")?,
            BluetoothMessage::AlreadyPaired => {
                ui.output("Already BLE paired with Bluetooth device");
                // TODO: this is an ugly edge case, but redoing it to look for either might be equally ugly
                if looking_for == BluetoothMessage::PairSuccess || discriminant(&looking_for) == discriminant(&BluetoothMessage::Pin("".to_string())) {
                    return Ok(msg);
                }
            }
            BluetoothMessage::UserCanceled => Err("User canceled.")?,
            BluetoothMessage::StartedAdvertising => {
                ui.output("Started advertising Bluetooth service")
            }
            BluetoothMessage::PeerOS(ref os) => ui.output(&format!("Peer's OS is {}", os)),
            BluetoothMessage::SSID(ref ssid) => ui.output(&format!("Peer's SSID is {}", ssid)),
            BluetoothMessage::Password(ref password) => {
                ui.output(&format!("Peer's password is {}", password))
            }
            BluetoothMessage::PeerReadSsid => ui.output("Peer read our SSID"),
            BluetoothMessage::PeerReadPassword => ui.output("Peer read our password"),
            BluetoothMessage::Other(ref s) => {
                ui.output(&format!("Bluetooth peering result: {}", s))
            }
        };
        if discriminant(&msg) == discriminant(&looking_for) {
            return Ok(msg);
        }
    }
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
