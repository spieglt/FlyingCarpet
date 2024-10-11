use std::{
    collections::HashMap,
    error::Error,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;
use windows::{
    core::{GUID, HSTRING},
    Devices::{
        Bluetooth::{
            Advertisement::{
                BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
                BluetoothLEAdvertisementWatcherStatus,
            },
            BluetoothLEDevice,
            GenericAttributeProfile::{
                GattCharacteristic, GattCommunicationStatus, GattDeviceService, GattWriteOption,
            },
        },
        Enumeration::{
            DeviceInformation, DeviceInformationCustomPairing, DevicePairingKinds,
            DevicePairingProtectionLevel, DevicePairingRequestedEventArgs,
            DevicePairingResultStatus,
        },
    },
    Foundation::{EventRegistrationToken, TypedEventHandler},
    Storage::Streams::{DataReader, DataWriter, UnicodeEncoding},
};

use crate::bluetooth::{SERVICE_UUID, SSID_CHARACTERISTIC_UUID};

use super::{BluetoothMessage, OS_CHARACTERISTIC_UUID, PASSWORD_CHARACTERISTIC_UUID};

type ScanCallback =
    TypedEventHandler<BluetoothLEAdvertisementWatcher, BluetoothLEAdvertisementReceivedEventArgs>;

pub(crate) struct BluetoothCentral {
    tx: mpsc::Sender<BluetoothMessage>,
    watcher: BluetoothLEAdvertisementWatcher,
    custom_pairing: Arc<Mutex<Option<DeviceInformationCustomPairing>>>,
    peer_device: Arc<tokio::sync::Mutex<Option<BluetoothLEDevice>>>,
    peer_service: Option<GattDeviceService>,
    characteristics: HashMap<String, Option<GattCharacteristic>>,
    scan_callback_token: Option<EventRegistrationToken>,
    pair_callback_token: Arc<Mutex<Option<EventRegistrationToken>>>,
}

impl BluetoothCentral {
    pub fn new(tx: mpsc::Sender<BluetoothMessage>) -> windows::core::Result<Self> {
        let mut characteristics = HashMap::new();
        characteristics.insert(OS_CHARACTERISTIC_UUID.to_string(), None);
        characteristics.insert(SSID_CHARACTERISTIC_UUID.to_string(), None);
        characteristics.insert(PASSWORD_CHARACTERISTIC_UUID.to_string(), None);
        Ok(BluetoothCentral {
            tx,
            watcher: BluetoothLEAdvertisementWatcher::new()?,
            custom_pairing: Arc::new(Mutex::new(None)),
            peer_device: Arc::new(tokio::sync::Mutex::new(None)),
            peer_service: None,
            characteristics,
            scan_callback_token: None,
            pair_callback_token: Arc::new(Mutex::new(None)),
        })
    }

    // start thread to send on tx, return handle to thread and rx?
    pub fn scan(&mut self, ble_ui_rx: mpsc::Receiver<bool>) -> windows::core::Result<()> {
        let ble_ui_rx = Arc::new(Mutex::new(ble_ui_rx));
        let thread_peer_device = self.peer_device.clone();
        let thread_tx = self.tx.clone();
        // let thread_scan_callback_token = self.scan_callback_token.clone();
        let thread_custom_pairing = self.custom_pairing.clone();
        let thread_pair_callback_token = self.pair_callback_token.clone();
        let received_handler = ScanCallback::new(move |_watcher, received_event_args| {
            let received_event_args = received_event_args
                .as_ref()
                .expect("Could not get received_event_args.");
            let advertisement = received_event_args
                .Advertisement()
                .expect("Could not get Advertisement from received_event_args.");
            let address = received_event_args.BluetoothAddress()?; // TODO: write scan result failed to tx if the ?s in this function fail?
            let service_uuids = advertisement.ServiceUuids()?;
            for uuid in service_uuids {
                if uuid == GUID::from(SERVICE_UUID) {
                    // device is advertising flying carpet service, pair with it
                    println!("found bluetooth {:12x}", address);
                    let device = BluetoothLEDevice::FromBluetoothAddressAsync(address)?.get()?;
                    let info = device
                        .DeviceInformation()
                        .expect("Could not get DeviceInformation for peer peripheral.");
                    let mut peer_device = thread_peer_device
                        .blocking_lock();
                        // .expect("Could not lock peer device mutex.");
                    *peer_device = Some(device);
                    BluetoothCentral::pair_device(&info, ble_ui_rx.clone(), thread_tx.clone(), thread_custom_pairing.clone(), thread_pair_callback_token.clone())?;
                    // TODO: pairing callback is running before these are set? pass thread_custom_pairing and thread_pair_callback_token in, set them there?
                    // let mut pairing = thread_custom_pairing
                    //     .lock()
                    //     .expect("Couldn't lock custom pairing");
                    // *pairing = Some(custom_pairing);
                    // let mut token = thread_pair_callback_token
                    //     .lock()
                    //     .expect("Couldn't lock callback token mutex");
                    // *token = Some(pair_callback_token);
                }
            }
            Ok(())
        });
        let scan_callback_token = self.watcher.Received(&received_handler)?;
        self.scan_callback_token = Some(scan_callback_token);
        self.watcher.Start()?;
        // TODO: ble_ui_rx.recv() here? we've started scan, which when we find a device will pair with it, which will send the PIN to js,
        // which will tauri.invoke() the user's choice and send the result on ble_ui_tx... but if we never find a device, will we block here indefinitely?
        // no, because it's a tokio channel? test. but this is not the right place to do this because we need to know javascript's answer before we accept the pair attempt.
        Ok(())
    }

    pub fn pair_device(
        device_info: &DeviceInformation,
        ble_ui_rx: Arc<Mutex<mpsc::Receiver<bool>>>,
        tx: mpsc::Sender<BluetoothMessage>,
        out_custom_pairing: Arc<Mutex<Option<DeviceInformationCustomPairing>>>,
        out_pair_callback_token: Arc<Mutex<Option<EventRegistrationToken>>>,
    ) -> windows::core::Result<()> {
        let thread_tx = tx.clone();
        println!("Pairing {}", device_info.Name()?);
        let pairing = device_info.Pairing()?;
        let custom_pairing = pairing.Custom()?;
        {
            // put this in braces so it doesn't hold the lock?
            let mut out_custom_pairing = out_custom_pairing.lock().expect("Could not lock custom pairing mutex");
            *out_custom_pairing = Some(custom_pairing.clone());
        }
        let pairing_handler = TypedEventHandler::<
            DeviceInformationCustomPairing,
            DevicePairingRequestedEventArgs,
        >::new(move |_custom_pairing, _event_args| {
            println!("Custom pairing requested");
            let args = _event_args.clone().unwrap();
            let pin = args.Pin()?.to_string();
            // TODO: emit this pin to js
            match thread_tx.blocking_send(BluetoothMessage::Pin(pin)) {
                Ok(()) => (),
                Err(e) => println!("Could not send on Bluetooth tx: {}", e),
            }
            // TODO: we need to receive javascript's answer here... which means we need ble_ui_rx here, which means we can't use it from the struct and clone it, which means we have to wrap it in an arc<mutex>?
            let approved = ble_ui_rx
                .lock()
                .expect("Could not lock ble_ui_rx mutex.")
                .blocking_recv()
                .expect("ble_ui_rx reply from js was None");
            if approved {
                args.Accept()?;
                thread_tx
                    .blocking_send(BluetoothMessage::PairSuccess)
                    .expect("Could not send on Bluetooth tx");
            } else {
                thread_tx
                    .blocking_send(BluetoothMessage::UserCanceled)
                    .expect("Could not send on Bluetooth tx");
            }
            Ok(())
        });
        let pair_callback_token = custom_pairing.PairingRequested(&pairing_handler)?;
        {
            // put this in braces so it doesn't hold the lock?
            let mut out_pair_callback_token = out_pair_callback_token.lock().expect("Could not lock pair callback token mutex");
            *out_pair_callback_token = Some(pair_callback_token);
        }
        let result = custom_pairing
            .PairWithProtectionLevelAsync(
                DevicePairingKinds::ConfirmPinMatch,
                DevicePairingProtectionLevel::EncryptionAndAuthentication,
            )?
            .get()?;

        let status = result.Status()?;
        let errors = HashMap::from(ERRORS);
        let error_msg = errors
            .get(&status.0)
            .expect("Could not find status in error map");
        println!("Pairing result: {}", error_msg);

        let res = tx.blocking_send(if status == DevicePairingResultStatus::AlreadyPaired {
            BluetoothMessage::AlreadyPaired
        } else {
            BluetoothMessage::Other(error_msg.to_string())
        });
        if res.is_err() {
            println!(
                "pair_device() was called but transfer thread has stopped listening: {}",
                res.unwrap_err()
            );
        }
        Ok(())
    }

    pub fn stop_watching(&self) -> windows::core::Result<()> {
        let status = self.watcher.Status()?;
        if status == BluetoothLEAdvertisementWatcherStatus::Started {
            println!("stopping watcher");
            self.watcher.Stop()?;
            println!("watcher is stopped");
            let pairing = self
                .custom_pairing
                .lock() // TODO: couldn't lock custom pairing because it's still open in pair_device()?
                .expect("Could not lock custom pairing mutex");
            let pairing = pairing.as_ref();
            println!("pairing.as_ref(): {:?}", pairing);
            match pairing {
                Some(custom_pairing) => {
                    println!("custom_pairing is Some");
                    let pct = self
                        .pair_callback_token
                        .lock()
                        .expect("Could not lock callback mutex");
                    if let Some(pair_callback_token) = *pct {
                        println!("pair_callback_token is Some");
                        custom_pairing.RemovePairingRequested(pair_callback_token)?;
                    }
                }
                None => (),
            }
            if let Some(scan_callback_token) = self.scan_callback_token {
                println!("watcher is Some");
                self.watcher.RemoveReceived(scan_callback_token)
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    pub async fn get_services_and_characteristics(&mut self) -> Result<(), Box<dyn Error>> {
        // read service
        let device = self.peer_device.blocking_lock();
        let device = device.as_ref().expect("Bluetooth central had no remote device");

        'outer: loop {
            tokio::task::yield_now().await;
            let services = device.GetGattServicesAsync()?.get()?.Services()?;
            for service in services {
                println!("UUID: {:?}", service.Uuid()?);
                if service.Uuid()? == GUID::from(SERVICE_UUID) {
                    println!("found service");
                    // let x = device.RequestAccessAsync()?.await?;
                    // println!("{:?}", x);
                    // println!("requested access");
                    for characteristic in [
                        OS_CHARACTERISTIC_UUID,
                        SSID_CHARACTERISTIC_UUID,
                        PASSWORD_CHARACTERISTIC_UUID,
                    ] {
                        let characteristics = service
                            .GetCharacteristicsForUuidAsync(GUID::from(characteristic))?
                            .get()?
                            .Characteristics()?;
                        println!("got chars");
                        for c in characteristics {
                            if c.Uuid()? == GUID::from(characteristic) {
                                self.characteristics
                                    .insert(characteristic.to_string(), Some(c));
                            }
                        }
                    }
                    self.peer_service = Some(service);
                    break 'outer;
                }
            }
            println!("did not find flying carpet service, trying again...");
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        // TODO: exiting this function without setting OS_CHARACTERISTIC_UUID
        // loop until we have all 3?

        // problem where if we hit pair on iOS first, windows sees flying carpet service. but if windows pairs first, we don't: solved by adding service to peripheralManager when it's powered on on iOS?
        // also required solving by removing the addition of the service from the central's poweredOn branch. if this was done first, before the peripheralManager was powered on, it would throw an API error and not advertise properly.
        // this happened inconsistently, based on whether the iOS central or peripheral came up first, which made debugging confusing.

        // next problem: we can't yield to a cancel in here because of "can't send MutexGuard<Option<windows::BluetoothLEDevice>>"-type errors.
        // fixed by changing the peer_device from std::sync::Mutex to tokio::sync::Mutex and using .blocking_lock() in the windows callbacks that can't be async.
        // is this loop totally necessary now that we've bug where iOS was setting the service on peripheralManager in the wrong place (in the central) and thus preventing the service from being advertised correctly?
        // don't know, but might as well keep it, don't think a retry here hurts anything.
        Ok(())
    }

    pub async fn read(&mut self, characteristic_uuid: &str) -> windows::core::Result<String> {
        let characteristic = self.characteristics[characteristic_uuid]
            .as_ref()
            .expect(&format!("Missing characteristic {}", characteristic_uuid));
        let i_buffer = characteristic.ReadValueAsync()?.get()?.Value()?;
        let size = i_buffer.Capacity()?;
        let data_reader = DataReader::FromBuffer(&i_buffer)?;
        data_reader.SetUnicodeEncoding(UnicodeEncoding::Utf8)?;
        let data_string = data_reader.ReadString(size)?.to_string();
        println!("IBuffer contents: {:?}", data_string);
        Ok(data_string)
    }

    pub async fn write(
        &mut self,
        characteristic_uuid: &str,
        value: &str,
    ) -> Result<(), Box<dyn Error>> {
        println!("value: {}", value);
        let characteristic = self.characteristics[characteristic_uuid]
            .as_ref()
            .expect(&format!("Missing characteristic {}", characteristic_uuid));
        let write_option = GattWriteOption::WriteWithResponse;
        let data_writer = DataWriter::new()?;
        let bytes_written = data_writer.WriteString(&HSTRING::from(value))?; // TODO: is this utf-8? WriteBytes instead?
        println!("bytes written: {}", bytes_written);
        let i_buffer = data_writer.DetachBuffer()?;
        let status = characteristic
            .WriteValueWithOptionAsync(&i_buffer, write_option)?
            .get()?;
        if status != GattCommunicationStatus::Success {
            Err(format!(
                "Error writing to Bluetooth peripheral: {:?}",
                status
            ))?;
        }
        Ok(())
    }
}

const ERRORS: [(i32, &str); 20] = [
    (DevicePairingResultStatus::Paired.0, "Paired"),
    (
        DevicePairingResultStatus::NotReadyToPair.0,
        "NotReadyToPair",
    ),
    (DevicePairingResultStatus::NotPaired.0, "NotPaired"),
    (DevicePairingResultStatus::AlreadyPaired.0, "AlreadyPaired"),
    (
        DevicePairingResultStatus::ConnectionRejected.0,
        "ConnectionRejected",
    ),
    (
        DevicePairingResultStatus::TooManyConnections.0,
        "TooManyConnections",
    ),
    (
        DevicePairingResultStatus::HardwareFailure.0,
        "HardwareFailure",
    ),
    (
        DevicePairingResultStatus::AuthenticationTimeout.0,
        "AuthenticationTimeout",
    ),
    (
        DevicePairingResultStatus::AuthenticationNotAllowed.0,
        "AuthenticationNotAllowed",
    ),
    (
        DevicePairingResultStatus::AuthenticationFailure.0,
        "AuthenticationFailure",
    ),
    (
        DevicePairingResultStatus::NoSupportedProfiles.0,
        "NoSupportedProfiles",
    ),
    (
        DevicePairingResultStatus::ProtectionLevelCouldNotBeMet.0,
        "ProtectionLevelCouldNotBeMet",
    ),
    (DevicePairingResultStatus::AccessDenied.0, "AccessDenied"),
    (
        DevicePairingResultStatus::InvalidCeremonyData.0,
        "InvalidCeremonyData",
    ),
    (
        DevicePairingResultStatus::PairingCanceled.0,
        "PairingCanceled",
    ),
    (
        DevicePairingResultStatus::OperationAlreadyInProgress.0,
        "OperationAlreadyInProgress",
    ),
    (
        DevicePairingResultStatus::RequiredHandlerNotRegistered.0,
        "RequiredHandlerNotRegistered",
    ),
    (
        DevicePairingResultStatus::RejectedByHandler.0,
        "RejectedByHandler",
    ),
    (
        DevicePairingResultStatus::RemoteDeviceHasAssociation.0,
        "RemoteDeviceHasAssociation",
    ),
    (DevicePairingResultStatus::Failed.0, "Failed"),
];
