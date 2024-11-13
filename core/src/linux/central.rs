use bluer::{gatt::{local::characteristic_control, remote::Characteristic}, Adapter, AdapterEvent, Device, DiscoveryFilter, DiscoveryTransport, Result, Uuid};
use std::{collections::{HashMap, HashSet}, time::Duration};
use tokio::{sync::mpsc, time::sleep};
use futures::{pin_mut, StreamExt};

use super::SERVICE_UUID;
use crate::{bluetooth::{OS_CHARACTERISTIC_UUID, PASSWORD_CHARACTERISTIC_UUID, SSID_CHARACTERISTIC_UUID}, utils::BluetoothMessage};

// pub(crate) struct BluetoothCentral {
//     tx: mpsc::Sender<BluetoothMessage>,
// }

// impl BluetoothCentral {
//     pub fn new(tx: mpsc::Sender<BluetoothMessage>) -> Result<Self> {
//         Ok(BluetoothCentral {
//             tx,
//         })
//     }

//     pub async fn scan(&mut self) -> bluer::Result<()> {
//         let mut uuids = HashSet::new();
//         uuids.insert(Uuid::parse_str(SERVICE_UUID).expect("Could not parse service UUID"));

//         let filter = DiscoveryFilter {
//             transport: DiscoveryTransport::Le,
//             uuids,
//             ..Default::default()
//         };
//         adapter.set_discovery_filter(filter).await?;
//         println!("Using discovery filter:\n{:#?}\n\n", adapter.discovery_filter().await);
//         Ok(())
//     }
// }

async fn find_charcteristics(device: &Device) -> Result<Option<HashMap<&str, Characteristic>>> {
    let addr = device.address();
    let uuids = device.uuids().await?.unwrap_or_default();

    let os_characteristic_uuid = Uuid::parse_str(OS_CHARACTERISTIC_UUID).unwrap();
    let ssid_characteristic_uuid = Uuid::parse_str(SSID_CHARACTERISTIC_UUID).unwrap();
    let password_characteristic_uuid = Uuid::parse_str(PASSWORD_CHARACTERISTIC_UUID).unwrap();
    println!("Discovered device {} with service UUIDs {:?}", addr, &uuids);
    let md = device.manufacturer_data().await?;
    println!("    Manufacturer data: {:x?}", &md);

    if uuids.contains(&Uuid::parse_str(SERVICE_UUID).unwrap()) {
        println!("    Device provides our service!");
        let mut characteristics = HashMap::new();

        sleep(Duration::from_secs(2)).await;
        if !device.is_connected().await? {
            println!("    Connecting...");
            let mut retries = 2;
            loop {
                match device.connect().await {
                    Ok(()) => break,
                    Err(err) if retries > 0 => {
                        println!("    Connect error: {}", &err);
                        retries -= 1;
                    }
                    Err(err) => return Err(err),
                }
            }

            println!("    Connected");
        } else {
            println!("    Already connected");
        }

        // TODO: bond
        // device.pair().await?;

        println!("    Enumerating services...");
        for service in device.services().await? {
            let uuid = service.uuid().await?;
            println!("    Service UUID: {}", &uuid);
            println!("    Service data: {:?}", service.all_properties().await?);
            if uuid == Uuid::parse_str(SERVICE_UUID).unwrap() {
                println!("    Found our service!");
                for char in service.characteristics().await? {
                    let uuid = char.uuid().await?;
                    println!("    Characteristic UUID: {}", &uuid);
                    println!("    Characteristic data: {:?}", char.all_properties().await?);
                    if uuid == os_characteristic_uuid {
                        characteristics.insert(OS_CHARACTERISTIC_UUID, char);
                        println!("found OS characteristic")
                    } else if uuid == ssid_characteristic_uuid {
                        characteristics.insert(SSID_CHARACTERISTIC_UUID, char);
                        println!("found ssid characteristic")
                    } else if uuid == password_characteristic_uuid {
                        characteristics.insert(PASSWORD_CHARACTERISTIC_UUID, char);
                        println!("found password characteristic")
                    }
                }
            }
        }
        return Ok(Some(characteristics));
    }

    Ok(None)
}

pub async fn scan(adapter: Adapter) -> bluer::Result<()> {
    let mut uuids = HashSet::new();
    uuids.insert(Uuid::parse_str(SERVICE_UUID).expect("Could not parse service UUID"));

    let filter = DiscoveryFilter {
        transport: DiscoveryTransport::Le,
        uuids,
        ..Default::default()
    };
    adapter.set_discovery_filter(filter).await?;
    println!(
        "Using discovery filter:\n{:#?}\n\n",
        adapter.discovery_filter().await
    );

    {
        println!(
            "Discovering on Bluetooth adapter {} with address {}\n",
            adapter.name(),
            adapter.address().await?
        );
        let discover = adapter.discover_devices().await?;
        pin_mut!(discover);
        let mut done = false;
        while let Some(evt) = discover.next().await {
            match evt {
                AdapterEvent::DeviceAdded(addr) => {
                    let device = adapter.device(addr)?;
                    match find_charcteristics(&device).await {
                        Ok(Some(characteristics)) => {
                            let os_char = &characteristics.get(OS_CHARACTERISTIC_UUID);
                            // let value = os_char.read().await?;
                            // let peer_os = String::from_utf8(value).expect("Peer OS value was not utf-8");
                        },
                        Ok(None) => (),
                        Err(err) => {
                            println!("    Device failed: {}", &err);
                            let _ = adapter.remove_device(device.address()).await;
                        }
                    }
                    match device.disconnect().await {
                        Ok(()) => println!("    Device disconnected"),
                        Err(err) => println!("    Device disconnection failed: {}", &err),
                    }
                    println!();
                }
                AdapterEvent::DeviceRemoved(addr) => {
                    println!("Device removed {addr}");
                }
                _ => (),
            }
            if done {
                break;
            }
        }
        println!("Stopping discovery");
    }

    Ok(())
}
