use std::{
    collections::HashMap,
    error::Error,
    io::stdin,
    sync::mpsc::Sender,
};
use windows::{
    core::{Interface, GUID, HSTRING},
    Devices::{
        Bluetooth::{
            Advertisement::{
                BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
            },
            BluetoothAdapter,
        },
        Enumeration::{
            DeviceInformation, DeviceInformationCustomPairing, DeviceInformationKind,
            DeviceInformationUpdate, DevicePairingKinds, DevicePairingRequestedEventArgs,
            DevicePairingResultStatus, DeviceWatcher,
        },
    },
    Foundation::{Collections::IIterable, IReference, TypedEventHandler},
};

use crate::bluetooth::SERVICE_UUID;

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

// start thread to send on tx, return handle to thread and rx?
pub fn watch_for_advertisements(
    tx: Sender<u64>,
) -> Result<BluetoothLEAdvertisementWatcher, Box<dyn Error>> {
    let watcher = BluetoothLEAdvertisementWatcher::new()?;

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
                tx.send(address).unwrap();
            }
        }
        Ok(())
    });
    watcher.Received(&received_handler)?;
    watcher.Start()?;
    Ok(watcher)
}

// pub fn stop_watching(watcher: BluetoothLEAdvertisementWatcher) -> Result<(), Box<dyn Error>> {
//     watcher.Stop();
//     Ok(())
// }

pub fn pair_device(device_info: &DeviceInformation) -> Result<(), Box<dyn Error>> {
    println!("Pairing {}", device_info.Name()?);
    // let result = device_info.Pairing()?.PairAsync()?.get()?;
    let pairing = device_info.Pairing()?;
    let custom_pairing = pairing.Custom()?;
    let pairing_handler = TypedEventHandler::<
        DeviceInformationCustomPairing,
        DevicePairingRequestedEventArgs,
    >::new(|_custom_pairing, _event_args| {
        println!("Custom pairing requested");
        let args = _event_args.clone().unwrap();
        let pin = args.Pin()?.to_string();
        println!("Does this pin match? Y/N: {}", pin);
        let mut user_input = String::new();
        match stdin().read_line(&mut user_input) {
            Ok(_n) => (),
            Err(e) => {
                println!("Could not read input: {e}");
                return Ok(())
            }
        };
        if user_input.chars().nth(0) == Some('Y') || user_input.chars().nth(0) == Some('y') {
            args.Accept()?;
        } else {
            println!("nope");
        }
        Ok(())
    });
    let _reg_token = custom_pairing.PairingRequested(&pairing_handler)?;
    let result = custom_pairing
        .PairAsync(DevicePairingKinds::ConfirmPinMatch)?
        .get()?;
    // let result = pairing
    //     .PairWithProtectionLevelAsync(
    //         DevicePairingProtectionLevel::Encryption
    //     )?
    //     .get()?;
    // let result = pairing.PairAsync()?.get()?;
    // println!("paired");

    let status = result.Status()?;
    let errors = HashMap::from(ERRORS);
    println!("Pairing result: {}", errors.get(&status.0).unwrap());
    Ok(())
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
