use bluer::{
    gatt::{remote::{Characteristic, CharacteristicWriteRequest}, WriteOp}, Adapter, AdapterEvent, Device, DiscoveryFilter, DiscoveryTransport, Error, ErrorKind, Result, Uuid
};
use futures::{pin_mut, StreamExt};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::time::sleep;

use super::SERVICE_UUID;
use crate::{
    bluetooth::{
        OS, OS_CHARACTERISTIC_UUID, PASSWORD_CHARACTERISTIC_UUID, SSID_CHARACTERISTIC_UUID,
    },
    network::is_hosting,
    utils::{generate_password, get_key_and_ssid},
    Mode, Peer,
};

pub async fn find_characteristics(device: &Device) -> Result<HashMap<&str, Characteristic>> {
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
            Err(Error{
                kind: ErrorKind::AlreadyConnected,
                message: "Already connected".to_string(),
            })?
        }

        // bond?
        // sleep(Duration::from_secs(2)).await;
        // if !device.is_paired().await? {
        //     println!("    Pairing...");
        //     let mut retries = 2;
        //     loop {
        //         match device.pair().await {
        //             Ok(()) => break,
        //             Err(err) if retries > 0 => {
        //                 println!("    Pair error: {}", &err);
        //                 retries -= 1;
        //             }
        //             Err(err) => return Err(err),
        //         }
        //     }
        //     println!("    Paired");
        // } else {
        //     println!("    Already paired");
        // }

        // sleep(Duration::from_secs(2)).await;
        // println!("    Enumerating services...");
        // if !device.is_services_resolved().await? {
        //     println!("Not resolved...");
        //     sleep(Duration::from_secs(2)).await;
        // } else {
        //     println!("Services are resolved: {:?}", device.services().await?);
        //     let data = device.service_data().await?;
        //     println!("Data: {:?}", data);
        // }
        // let mut events = device.events().await.unwrap();
        // while let Some(ev) = events.next().await {
        //     println!("Received event {:?}", ev);
        // }
        for service in device.services().await? {
            let uuid = service.uuid().await?;
            println!("    Service UUID: {}", &uuid);
            println!("    Service data: {:?}", service.all_properties().await?);
            if uuid == Uuid::parse_str(SERVICE_UUID).unwrap() {
                println!("    Found our service!");
                for char in service.characteristics().await? {
                    let uuid = char.uuid().await?;
                    println!("    Characteristic UUID: {}", &uuid);
                    println!(
                        "    Characteristic data: {:?}",
                        char.all_properties().await?
                    );
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

        if characteristics.contains_key(OS_CHARACTERISTIC_UUID)
            && characteristics.contains_key(SSID_CHARACTERISTIC_UUID)
            && characteristics.contains_key(PASSWORD_CHARACTERISTIC_UUID)
        {
            Ok(characteristics)
        } else {
            let e = bluer::Error {
                kind: bluer::ErrorKind::ServicesUnresolved,
                message: "Did not read all Flying Carpet characteristics from peer.".to_string(),
            };
            Err(e)
        }
    } else {
        let err = bluer::Error {
            kind: ErrorKind::ServicesUnresolved,
            message: "Could not find service UUID on scanned device".to_string(),
        };
        Err(err)
    }
}

pub async fn scan(adapter: &Adapter) -> bluer::Result<Device> {
    let mut uuids = HashSet::new();
    uuids.insert(Uuid::parse_str(SERVICE_UUID).expect("Could not parse service UUID"));

    let filter = DiscoveryFilter {
        transport: DiscoveryTransport::Auto,
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
        while let Some(evt) = discover.next().await {
            match evt {
                AdapterEvent::DeviceAdded(addr) => {
                    // let device = adapter.connect_device(addr, bluer::AddressType::LePublic).await?;
                    let device = adapter.device(addr)?;
                    return Ok(device);
                    // match device.disconnect().await {
                    //     Ok(()) => println!("    Device disconnected"),
                    //     Err(err) => println!("    Device disconnection failed: {}", &err),
                    // }
                    // println!();
                }
                AdapterEvent::DeviceRemoved(addr) => {
                    println!("Device removed {addr}");
                }
                other_event => println!("Processed other event: {:?}", other_event),
            }
        }
        println!("Stopping discovery");
    }
    Err(bluer::Error {
        kind: ErrorKind::NotFound,
        message: "Exited scan() without finding device".to_string(),
    })
}

pub async fn exchange_info(
    characteristics: HashMap<&str, Characteristic>,
    mode: &Mode,
) -> bluer::Result<(String, String, String)> {
    // have to use this with write_ext() for the write requests: iOS wouldn't receive unconfirmed writes, which WriteOp::Request provides.
    // not sure if iOS requires it or if i did somehow. bluer seems to default to WriteOp::Command which has no confirmation.
    let write_req = CharacteristicWriteRequest {
        offset: 0,
        op_type: WriteOp::Request,
        prepare_authorize: true,
        ..Default::default()
    };

    // read peer's OS
    let os_char = &characteristics[OS_CHARACTERISTIC_UUID];
    let value = os_char.read().await?;
    let peer_os = String::from_utf8(value).expect("Peer OS value was not utf-8");
    println!("Peer OS: {}", peer_os);
    sleep(Duration::from_secs(1)).await;
    // write our OS
    os_char.write_ext(OS.as_bytes(), &write_req).await?;
    println!("Wrote OS to peer");
    sleep(Duration::from_secs(1)).await;

    let ssid_char = &characteristics[SSID_CHARACTERISTIC_UUID];
    let password_char = &characteristics[PASSWORD_CHARACTERISTIC_UUID];
    if is_hosting(&Peer::from(peer_os.as_str()), mode) {
        // write ssid and password
        let password = generate_password();
        let (_, ssid) = get_key_and_ssid(&password);
        ssid_char.write_ext(ssid.as_bytes(), &write_req).await?;
        // let CharacteristicWriteRequest
        // ssid_char.write_ext(value, req);
        println!("Wrote SSID to peer");
        sleep(Duration::from_secs(1)).await;
        password_char
            .write_ext(password.as_bytes(), &write_req)
            .await?;
        println!("Wrote password to peer");
        sleep(Duration::from_secs(1)).await;
        Ok((peer_os, ssid, password))
    } else {
        // read ssid and password
        let ssid = ssid_char.read().await?;
        let ssid = String::from_utf8(ssid).expect("SSID was not UTF-8");
        println!("Peer's SSID: {}", ssid);
        let password = password_char.read().await?;
        let password = String::from_utf8(password).expect("Password was not UTF-8");
        println!("Peer's password: {}", password);
        Ok((peer_os, ssid, password))
    }
}
