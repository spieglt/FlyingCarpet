mod central;
mod peripheral;

use crate::{
    error::{fc_error, FCError},
    network::{self, is_hosting},
    utils::{generate_password, get_key_and_ssid, BluetoothMessage},
    Mode, Peer, UI,
};
use central::BluetoothCentral;
use peripheral::BluetoothPeripheral;
use std::mem::discriminant;
use tokio::{sync::mpsc, time};
use windows::{
    core::HSTRING,
    Devices::{Bluetooth::BluetoothAdapter, Radios::RadioState},
    Storage::Streams::{DataReader, DataWriter, IBuffer, UnicodeEncoding},
};

impl From<windows::core::Error> for FCError {
    fn from(value: windows::core::Error) -> Self {
        FCError {
            message: format!("Windows error: {}", value),
        }
    }
}

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

// central goes scan -> bond -> connect -> discoverServices -> read OS -> write OS
// -> connectToPeer -> start hotspot and write ssid/pw, or read ssid/pw and join hotspot

// peripheral goes advertise, wait for bonding, wait for OS read, wait for OS write,
// connectToPeer, start hotspot and wait for ssid/password to be read, or wait for ssid/pw writes and joinHotspot

pub async fn check_support() -> Result<(), FCError> {
    let adapter = BluetoothAdapter::GetDefaultAsync()?.get()?;
    println!("got adapter");
    let radio = adapter.GetRadioAsync()?.get()?;
    println!("got radio");
    if radio.State()? != RadioState::On {
        fc_error("radio is not on")?;
    }
    if !adapter.IsCentralRoleSupported()? {
        fc_error("central role not supported")?;
    }
    println!("Central role is supported");
    if !adapter.IsPeripheralRoleSupported()? {
        fc_error("peripheral role not supported")?;
    }
    println!("Peripheral role is supported");
    Ok(())
}

pub async fn negotiate_bluetooth<T: UI>(
    mode: &Mode,
    ble_ui_rx: mpsc::Receiver<bool>,
    ui: &T,
) -> Result<(String, String, String), FCError> {
    let (tx, mut rx) = mpsc::channel(1);
    let mut peripheral = BluetoothPeripheral::new(tx.clone())?;
    let mut central = BluetoothCentral::new(tx.clone())?;
    if let Mode::Send(_) = mode {
        // acting as peripheral
        ui.output("Advertising Bluetooth service...");
        peripheral.add_characteristics()?;
        peripheral.start_advertising()?;

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
            fc_error(&format!(
                "Peripheral received incorrect BluetoothMessage. Expected peer OS, got {:?}",
                msg
            ))?;
        }

        let peer = Peer::try_from(peer_os.as_str())?;
        let result = if is_hosting(&peer, mode) {
            let password = generate_password();
            let (_, ssid) = get_key_and_ssid(&password);
            {
                let mut peripheral_ssid = peripheral.ssid.lock().await;
                *peripheral_ssid = Some(ssid.clone());
                let mut peripheral_password = peripheral.password.lock().await;
                *peripheral_password = Some(password.clone());
            }
            println!("set peripheral ssid and password");
            println!("waiting for ssid to be read...");
            process_bluetooth_message(BluetoothMessage::PeerReadSsid, &mut rx, ui).await?;
            println!("waiting for password to be read...");
            process_bluetooth_message(BluetoothMessage::PeerReadPassword, &mut rx, ui).await?;
            (peer_os, ssid.clone(), password)
        } else {
            // if joining, receive writes
            // receive ssid
            let msg = process_bluetooth_message(BluetoothMessage::SSID(String::new()), &mut rx, ui)
                .await?;
            if let BluetoothMessage::SSID(ssid) = msg {
                peer_ssid = ssid;
            } else {
                fc_error(&format!(
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
                fc_error(&format!(
                    "Peripheral received incorrect BluetoothMessage. Expected password, got {:?}",
                    msg
                ))?;
            }
            // keep everything in scope until peer has had a chance to read the password
            time::sleep(time::Duration::from_secs(1)).await;
            (peer_os, peer_ssid, peer_password)
        };
        // Done exchanging OS/SSID/password. Stop advertising explicitly; the connected
        // central keeps its link, so this only stops new devices from discovering us.
        peripheral.stop_advertising()?;
        Ok(result)
    } else {
        // acting as central
        // scan for device advertising flying carpet service
        ui.output("Scanning for Bluetooth peripherals...");
        central.scan(ble_ui_rx)?;

        central.stop_watching()?;
        println!("stopped watching");

        // Windows sometimes can't enumerate GATT services of a device it's still bonded
        // to from an earlier transfer (E_UNEXPECTED, 0x8000FFFF), especially when the BLE
        // roles were reversed last time (we were the peripheral, so the bond predates the
        // peer hosting our service). Recovery ladder, cheapest rung first (background and
        // sources in docs/windows-ble-gatt-0x8000ffff.md):
        // 1. retry enumeration a couple of times ~1s apart — Microsoft's guidance for
        //    bonded LE-privacy peripherals, whose first connection can race the stack's
        //    internal RPA/IRK resolution;
        // 2. drop the bond, rescan, and pair fresh, the way a manual restart of the
        //    transfer would. One unpair only.
        // The attempt/timing/diagnostic output is deliberately chatty so field reports of
        // this failure show which rung fixed it (and whether rung 2 is ever needed).
        const ENUMERATION_ATTEMPTS: u32 = 3;
        let mut retried_after_unpair = false;
        'pairing: loop {
            // if we're looking for Pin or PairSuccess, process_bluetooth_message() will bail when it sees AlreadyPaired
            println!("waiting for callback...");
            let msg =
                process_bluetooth_message(BluetoothMessage::Pin("".to_string()), &mut rx, ui)
                    .await?;

            // wait to pair
            if msg != BluetoothMessage::AlreadyPaired {
                process_bluetooth_message(BluetoothMessage::PairSuccess, &mut rx, ui).await?;
            }

            // discover service and characteristics once paired
            let mut last_error = None;
            for attempt in 1..=ENUMERATION_ATTEMPTS {
                println!("before get_services_and_characteristics, attempt {}", attempt);
                let start = time::Instant::now();
                match central.get_services_and_characteristics().await {
                    Ok(()) => {
                        if attempt > 1 {
                            ui.output(&format!(
                                "Reading Bluetooth services succeeded on attempt {}",
                                attempt
                            ));
                        }
                        break 'pairing;
                    }
                    Err(e) => {
                        ui.output(&format!(
                            "Couldn't read Bluetooth services (attempt {}/{}, took {:.1}s): {}",
                            attempt,
                            ENUMERATION_ATTEMPTS,
                            start.elapsed().as_secs_f32(),
                            e
                        ));
                        if attempt == 1 {
                            ui.output(&format!(
                                "Diagnostic info: {} bond; peer {} connected when discovered",
                                if msg == BluetoothMessage::AlreadyPaired {
                                    "reused"
                                } else {
                                    "new"
                                },
                                if central.was_already_connected() {
                                    "was already"
                                } else {
                                    "was not"
                                },
                            ));
                        }
                        last_error = Some(e);
                        if attempt < ENUMERATION_ATTEMPTS {
                            time::sleep(time::Duration::from_secs(1)).await;
                        }
                    }
                }
            }
            let last_error = last_error.expect("enumeration attempts exhausted without an error");

            if msg == BluetoothMessage::AlreadyPaired && !retried_after_unpair {
                // The only unpair left in this file, and the only one with positive evidence
                // that the *bond* is the problem: we reused an existing bond and enumeration
                // failed every attempt. Unpairing is one-sided — the peer keeps its half and
                // we cannot tell it — so say so plainly, because that is the user's only way
                // out if the re-pair below also fails, and Apple peers cannot clear their
                // half programmatically at all.
                retried_after_unpair = true;
                ui.output(
                    "Couldn't read services of already-paired Bluetooth device. Unpairing and pairing again...",
                );
                ui.output(
                    "Note: this removes the pairing on this device only. If the transfer still fails, remove this device from the other device's Bluetooth settings as well, then try again.",
                );
                if let Err(unpair_error) = central.unpair().await {
                    println!("Error unpairing: {}", unpair_error);
                }
                central.rescan().await?;
            } else {
                // A fresh pairing that still can't enumerate is not a stale-bond problem, so
                // leave the bond alone and report the error.
                Err(last_error)?
            }
        }
        println!("after get_services_and_characteristics");

        // Nothing below unpairs on failure. Once enumeration has succeeded the bond is
        // demonstrably fine and the link is up, so a failing characteristic read or write is
        // a timing or peer-side problem — dropping the bond can't fix it, and it leaves the
        // peer holding a key we discarded, which is strictly worse for the next attempt (see
        // "never unpair unilaterally" in docs/bluetooth-field-guide.md). These six call sites
        // used to unpair; against an Apple peer that is unrecoverable without the user
        // removing the pairing in System Settings, because CoreBluetooth exposes no API to
        // clear its half.
        ui.output("Reading peer's OS");
        let peer = central.read(OS_CHARACTERISTIC_UUID).await?;
        ui.output(&format!("Peer OS: {:?}", peer));

        // write OS
        central.write(OS_CHARACTERISTIC_UUID, OS).await?;
        println!("wrote OS");

        // read or write ssid and password
        let peer_os = Peer::try_from(peer.as_str())?;
        let (ssid, password) = if network::is_hosting(&peer_os, mode) {
            println!("hosting, writing wifi info to peer");
            let password = generate_password();
            let (_, ssid) = get_key_and_ssid(&password);
            central.write(SSID_CHARACTERISTIC_UUID, &ssid).await?;
            central
                .write(PASSWORD_CHARACTERISTIC_UUID, &password)
                .await?;
            (ssid, password)
        } else {
            println!("joining, reading wifi info from peer");
            let ssid = central.read(SSID_CHARACTERISTIC_UUID).await?;
            let password = central.read(PASSWORD_CHARACTERISTIC_UUID).await?;
            (ssid, password)
        };
        // We stay paired after the transfer, like every other platform.
        Ok((peer, ssid, password))
    }
}

pub async fn process_bluetooth_message<T: UI>(
    looking_for: BluetoothMessage,
    rx: &mut mpsc::Receiver<BluetoothMessage>,
    ui: &T,
) -> Result<BluetoothMessage, FCError> {
    loop {
        println!("waiting for bluetooth message...");
        let msg = rx
            .recv()
            .await
            .expect("Bluetooth message channel unexpectedly closed.");
        println!("received {:?}", msg);
        match &msg {
            BluetoothMessage::Pin(pin) => {
                ui.show_pin(pin);
            }
            BluetoothMessage::PairApproved => ui.output("Pairing approved."),
            BluetoothMessage::PairSuccess => {
                // can use this to represent AlreadyPaired on windows? don't need to emit pin, just need to proceed.
                // and nothing will be blocked in central because the pairing_handler won't be called.
                ui.output("Successfully paired");
            }
            BluetoothMessage::PairFailure => fc_error("Pairing failed.")?,
            BluetoothMessage::AlreadyPaired => {
                ui.output("Already BLE paired with Bluetooth device");
                if looking_for == BluetoothMessage::PairSuccess
                    || discriminant(&looking_for)
                        == discriminant(&BluetoothMessage::Pin("".to_string()))
                {
                    return Ok(msg);
                }
            }
            BluetoothMessage::UserCanceled => fc_error("User canceled.")?,
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
            BluetoothMessage::OtherError(s) => fc_error(s.as_str())?,
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
    data_writer.SetUnicodeEncoding(UnicodeEncoding::Utf8)?;
    let bytes_written = data_writer.WriteString(&HSTRING::from(s))?;
    println!("bytes written to ibuffer: {}", bytes_written);
    Ok(data_writer.DetachBuffer()?)
}

// https://stackoverflow.com/a/38704180/9242143
