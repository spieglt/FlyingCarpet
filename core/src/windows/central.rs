use std::{
    collections::HashMap,
    error::Error,
    sync::{Arc, Mutex},
};
use windows::{
    core::GUID,
    Devices::{
        Bluetooth::{
            Advertisement::{
                BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
            },
            BluetoothAdapter, BluetoothLEDevice,
        },
        Enumeration::{DeviceInformation, DevicePairingProtectionLevel, DevicePairingResultStatus},
    },
    Foundation::TypedEventHandler, Storage::Streams::{DataReader, UnicodeEncoding},
};

use crate::bluetooth::{SERVICE_UUID, SSID_CHARACTERISTIC_UUID};

pub(crate) fn check_support() -> windows::core::Result<bool> {
    let local_adapter = BluetoothAdapter::GetDefaultAsync()?.get();
    Ok(if local_adapter.is_ok() {
        println!(
            "our address: {:12x}",
            local_adapter.clone().unwrap().BluetoothAddress().unwrap()
        );
        local_adapter.unwrap().IsCentralRoleSupported()?
    } else {
        false
    })
}

pub(crate) struct BluetoothCentral {
    watcher: BluetoothLEAdvertisementWatcher,
    peer_device: Arc<Mutex<Option<BluetoothLEDevice>>>,
}

impl BluetoothCentral {
    pub fn new() -> windows::core::Result<Self> {
        Ok(BluetoothCentral {
            watcher: BluetoothLEAdvertisementWatcher::new()?,
            peer_device: Arc::new(Mutex::new(None)),
        })
    }

    // start thread to send on tx, return handle to thread and rx?
    pub fn scan(&mut self) -> windows::core::Result<()> {
        let thread_peer_device = self.peer_device.clone();
        let received_handler = TypedEventHandler::<
            BluetoothLEAdvertisementWatcher,
            BluetoothLEAdvertisementReceivedEventArgs,
        >::new(move |_watcher, received_event_args| {
            let received_event_args = received_event_args.clone().unwrap();
            let advertisement = received_event_args.Advertisement().unwrap();
            let address = received_event_args.BluetoothAddress()?;
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
                    BluetoothCentral::pair_device(&info);
                }
            }
            Ok(())
        });
        self.watcher.Received(&received_handler)?;
        self.watcher.Start()?;
        Ok(())
    }

    // pub fn stop_watching(watcher: BluetoothLEAdvertisementWatcher) -> Result<(), Box<dyn Error>> {
    //     watcher.Stop();
    //     Ok(())
    // }

    pub fn pair_device(device_info: &DeviceInformation) -> Result<(), Box<dyn Error>> {
        println!("Pairing {}", device_info.Name()?);
        let pairing = device_info.Pairing()?;
        // let custom_pairing = pairing.Custom()?;
        // let pairing_handler = TypedEventHandler::<
        //     DeviceInformationCustomPairing,
        //     DevicePairingRequestedEventArgs,
        // >::new(|_custom_pairing, _event_args| {
        //     println!("Custom pairing requested");
        //     let args = _event_args.clone().unwrap();
        //     let pin = args.Pin()?.to_string();
        //     println!("Does this pin match? Y/N: {}", pin);
        //     let mut user_input = String::new();
        //     match stdin().read_line(&mut user_input) {
        //         Ok(_n) => (),
        //         Err(e) => {
        //             println!("Could not read input: {e}");
        //             return Ok(());
        //         }
        //     };
        //     if user_input.chars().nth(0) == Some('Y') || user_input.chars().nth(0) == Some('y') {
        //         args.Accept()?;
        //     } else {
        //         println!("nope");
        //     }
        //     Ok(())
        // });
        // let _reg_token = custom_pairing.PairingRequested(&pairing_handler)?;
        // let result = custom_pairing
        //     .PairAsync(DevicePairingKinds::ConfirmPinMatch)?
        //     .get()?;
        let result = pairing
            .PairWithProtectionLevelAsync(
                DevicePairingProtectionLevel::EncryptionAndAuthentication,
            )?
            .get()?;

        // let result = pairing.PairAsync()?.get()?;
        println!("paired");

        let status = result.Status()?;
        let errors = HashMap::from(ERRORS);
        println!("Pairing result: {}", errors.get(&status.0).unwrap());
        Ok(())
    }

    pub async fn read(&mut self) -> Result<(), Box<dyn Error>> {
        
        // read service
        let device = self.peer_device.lock();
        let device = device.as_ref().expect("Could not lock peer_device mutex");
        let device = device.as_ref().expect("Bluetooth central had no remote device");

        let services = device.GetGattServicesAsync()?.await?.Services()?;
        for service in services {
            println!("UUID: {:?}", service.Uuid()?);
            if service.Uuid()? == GUID::from(SERVICE_UUID) {
                println!("found service");
                // let x = device.RequestAccessAsync()?.await?;
                // println!("{:?}", x);
                // println!("requested access");
                let characteristics = service
                    .GetCharacteristicsForUuidAsync(GUID::from(SSID_CHARACTERISTIC_UUID))?
                    .await?
                    .Characteristics()?;
                println!("got chars");
                for characteristic in characteristics {
                    let i_buffer = characteristic.ReadValueAsync().ok().unwrap().await?.Value();
                    if i_buffer.is_err() {
                        println!("nothing in buffer");
                        continue;
                    }
                    let i_buffer = i_buffer.unwrap();
                    println!("IBuffer contents: {:?}", i_buffer);
                    let size = i_buffer.Capacity()?;
                    let data_reader = DataReader::FromBuffer(&i_buffer)?;
                    data_reader.SetUnicodeEncoding(UnicodeEncoding::Utf8)?;
                    let data_string = data_reader.ReadString(size)?.to_string();
                    println!("message: {}", data_string);
                }
            }
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
