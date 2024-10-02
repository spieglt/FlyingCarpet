use std::{
    collections::HashMap,
    error::Error,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;
use windows::{
    core::GUID,
    Devices::{
        Bluetooth::{
            Advertisement::{
                BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
            },
            BluetoothLEDevice,
            GenericAttributeProfile::{GattCharacteristic, GattDeviceService},
        },
        Enumeration::{
            DeviceInformation, DeviceInformationCustomPairing, DevicePairingKinds,
            DevicePairingProtectionLevel, DevicePairingRequestedEventArgs,
            DevicePairingResultStatus,
        },
    },
    Foundation::TypedEventHandler,
    Storage::Streams::{DataReader, UnicodeEncoding},
};

use crate::bluetooth::{SERVICE_UUID, SSID_CHARACTERISTIC_UUID};

use super::{BluetoothMessage, OS_CHARACTERISTIC_UUID, PASSWORD_CHARACTERISTIC_UUID};

pub(crate) struct BluetoothCentral {
    tx: mpsc::Sender<BluetoothMessage>,
    watcher: BluetoothLEAdvertisementWatcher,
    peer_device: Arc<Mutex<Option<BluetoothLEDevice>>>,
    peer_service: Option<GattDeviceService>,
    characteristics: HashMap<String, Option<GattCharacteristic>>,
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
            peer_device: Arc::new(Mutex::new(None)),
            peer_service: None,
            characteristics,
        })
    }

    // start thread to send on tx, return handle to thread and rx?
    pub fn scan(&mut self, ble_ui_rx: mpsc::Receiver<bool>) -> windows::core::Result<()> {
        let ble_ui_rx = Arc::new(Mutex::new(ble_ui_rx));
        let thread_peer_device = self.peer_device.clone();
        let thread_tx = self.tx.clone();
        let received_handler = TypedEventHandler::<
            BluetoothLEAdvertisementWatcher,
            BluetoothLEAdvertisementReceivedEventArgs,
        >::new(move |_watcher, received_event_args| {
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
                        .lock()
                        .expect("Could not lock peer device mutex.");
                    *peer_device = Some(device);
                    BluetoothCentral::pair_device(&info, ble_ui_rx.clone(), thread_tx.clone())?;
                }
            }
            Ok(())
        });
        self.watcher.Received(&received_handler)?;
        self.watcher.Start()?;
        // TODO: ble_ui_rx.recv() here? we've started scan, which when we find a device will pair with it, which will send the PIN to js,
        // which will tauri.invoke() the user's choice and send the result on ble_ui_tx... but if we never find a device, will we block here indefinitely?
        // no, because it's a tokio channel? test. but this is not the right place to do this because we need to know javascript's answer before we accept the pair attempt.
        Ok(())
    }

    // pub fn stop_watching(watcher: BluetoothLEAdvertisementWatcher) -> Result<(), Box<dyn Error>> {
    //     watcher.Stop();
    //     Ok(())
    // }

    pub fn pair_device(
        device_info: &DeviceInformation,
        ble_ui_rx: Arc<Mutex<mpsc::Receiver<bool>>>,
        tx: mpsc::Sender<BluetoothMessage>,
    ) -> windows::core::Result<()> {
        println!("Pairing {}", device_info.Name()?);
        let pairing = device_info.Pairing()?;

        let custom_pairing = pairing.Custom()?;
        let pairing_handler = TypedEventHandler::<
            DeviceInformationCustomPairing,
            DevicePairingRequestedEventArgs,
        >::new(move |_custom_pairing, _event_args| {
            println!("Custom pairing requested");
            let args = _event_args.clone().unwrap();
            let pin = args.Pin()?.to_string();
            // TODO: emit this pin to js
            tx.blocking_send(BluetoothMessage::Pin(pin))
                .expect("Could not send on Bluetooth tx");
            // TODO: we need to receive javascript's answer here... which means we need ble_ui_rx here, which means we can't use it from the struct and clone it, which means we have to wrap it in an arc<mutex>?
            let approved = ble_ui_rx
                .lock()
                .expect("Could not lock ble_ui_rx mutex.")
                .blocking_recv()
                .expect("ble_ui_rx reply from js was None");
            if approved {
                args.Accept()?;
            } else {
                println!("nope");
            }
            Ok(())
        });
        let _reg_token = custom_pairing.PairingRequested(&pairing_handler)?;
        let result = custom_pairing
            .PairWithProtectionLevelAsync(
                DevicePairingKinds::ConfirmPinMatch,
                DevicePairingProtectionLevel::EncryptionAndAuthentication,
            )?
            .get()?;

        let status = result.Status()?;
        let errors = HashMap::from(ERRORS);
        println!("Pairing result: {}", errors.get(&status.0).unwrap());
        Ok(())
    }

    pub async fn get_services_and_characteristics(&mut self) -> Result<(), Box<dyn Error>> {
        // read service
        let device = self.peer_device.lock();
        let device = device.as_ref().expect("Could not lock peer_device mutex");
        let device = device
            .as_ref()
            .expect("Bluetooth central had no remote device");

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
            }
        }
        Ok(())
    }

    pub async fn read(&mut self, characteristic_uuid: &str) -> Result<String, Box<dyn Error>> {
        let characteristic = self.characteristics[characteristic_uuid]
            .as_ref()
            .expect(&format!("Missing characteristic {}", characteristic_uuid));
        let i_buffer = characteristic.ReadValueAsync()?.get()?.Value()?;
        println!("IBuffer contents: {:?}", i_buffer);
        let size = i_buffer.Capacity()?;
        let data_reader = DataReader::FromBuffer(&i_buffer)?;
        data_reader.SetUnicodeEncoding(UnicodeEncoding::Utf8)?;
        let data_string = data_reader.ReadString(size)?.to_string();
        Ok(data_string)
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
