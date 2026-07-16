use bluer::{
    gatt::{
        remote::{Characteristic, CharacteristicWriteRequest},
        WriteOp,
    },
    l2cap::{Security, SecurityLevel, SeqPacket, Socket, SocketAddr},
    Adapter, AdapterEvent, Address, AddressType, Device, DiscoveryFilter, DiscoveryTransport,
    ErrorKind, Result, Uuid,
};
use futures::{pin_mut, StreamExt};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::time::{sleep, timeout};

use super::SERVICE_UUID;

// Dynamic LE PSM used only to trigger SMP pairing via a socket connection attempt; the
// connection itself is expected to be refused. See the bonding comment in
// find_characteristics.
const BOND_PSM: u16 = 0x0083;
use crate::{
    bluetooth::{
        OS, OS_CHARACTERISTIC_UUID, PASSWORD_CHARACTERISTIC_UUID, SSID_CHARACTERISTIC_UUID,
    },
    network::is_hosting,
    utils::{generate_password, get_key_and_ssid},
    Mode, Peer,
};

pub async fn find_characteristics(
    device: &Device,
) -> Result<HashMap<&'static str, Characteristic>> {
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

        // Bond over LE BEFORE letting BlueZ connect. macOS advertises with a PUBLIC address
        // and dual-mode flags; BlueZ then stamps both bearers' last-seen on every LE
        // advertisement (adapter.c update_found_devices) and its bearer tiebreak explicitly
        // prefers BR/EDR (device.c select_conn_bearer), so Device1.Connect()/Pair() on an
        // unbonded Mac ALWAYS page classic — which pairs over SSP but exposes no GATT.
        // The one rule that overrides the tiebreak is "prefer the bonded bearer when exactly
        // one is bonded": an LE-only bond makes Connect() dial LE permanently. We create that
        // bond here by opening an LE L2CAP socket with high security: the kernel brings up
        // the LE link and runs SMP pairing (numeric comparison, answered by our agent and
        // the dialog on the peer) before the connection attempt, which is then refused —
        // nothing listens on this PSM; the bond is the point. Random-address peers
        // (Windows/Android/iOS) always connect over LE anyway and keep the old behavior.
        if !device.is_paired().await?
            && device.address_type().await? == AddressType::LePublic
            && !device.is_connected().await?
        {
            println!("    Bonding over LE...");
            let socket = Socket::<SeqPacket>::new_seq_packet()?;
            socket.bind(SocketAddr::new(Address::any(), AddressType::LePublic, 0))?;
            socket.set_security(Security { level: SecurityLevel::High, key_size: 16 })?;
            let target = SocketAddr::new(addr, AddressType::LePublic, BOND_PSM);
            match timeout(Duration::from_secs(60), socket.connect(target)).await {
                Ok(Ok(_)) => println!("    LE bonding socket connected"),
                Ok(Err(e)) => println!("    LE bonding socket closed: {}", e),
                Err(_) => println!("    LE bonding socket timed out"),
            }
            if !device.is_paired().await? {
                return Err(bluer::Error {
                    kind: ErrorKind::AuthenticationFailed,
                    message: "LE pairing did not complete. Confirm the pairing dialog on the sending device and try again.".to_string(),
                });
            }
            println!("    LE bond established");
        }

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

        // Persist the bond so later transfers skip pairing (and macOS's cached keys keep
        // matching — the fix for the reverse-direction CBError 14 failure).
        if let Err(e) = device.set_trusted(true).await {
            println!("    Could not set device trusted: {}", e);
        }

        // macOS may only expose the Flying Carpet service once the link is encrypted,
        // re-publishing it via a Service Changed indication that makes BlueZ toggle
        // ServicesResolved and re-discover. Retry enumeration briefly rather than failing
        // on a first empty or partial read.
        let mut retries = 3;
        loop {
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
                return Ok(characteristics);
            }
            if retries == 0 {
                let e = bluer::Error {
                    kind: bluer::ErrorKind::ServicesUnresolved,
                    message: "Did not read all Flying Carpet characteristics from peer."
                        .to_string(),
                };
                return Err(e);
            }
            retries -= 1;
            println!("    Flying Carpet characteristics not all present yet, retrying...");
            sleep(Duration::from_secs(2)).await;
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
    let fc_uuid = Uuid::parse_str(SERVICE_UUID).expect("Could not parse service UUID");
    let mut uuids = HashSet::new();
    uuids.insert(fc_uuid);

    // bluer's discover_devices() pre-seeds its event stream with every device BlueZ already
    // knows, bypassing the discovery filter. A cached entry for the peer short-circuits the
    // scan before any live LE advertisement arrives, and Connect() then picks its bearer by
    // "bonded first, else most recently seen" — which after any classic-BT contact with a
    // dual-mode Mac means a BR/EDR page instead of an LE connection (br-connection-unknown).
    // Purge unpaired cached entries for our service so the peer must be rediscovered from a
    // live LE advertisement; bonded peers are kept (an LE bond makes Connect() pick LE).
    for addr in adapter.device_addresses().await? {
        let device = adapter.device(addr)?;
        if device.is_paired().await.unwrap_or(false) {
            continue;
        }
        let known_uuids = device.uuids().await.ok().flatten().unwrap_or_default();
        if known_uuids.contains(&fc_uuid) {
            println!(
                "Removing cached unpaired device {} so it can be rediscovered over LE",
                addr
            );
            if let Err(e) = adapter.remove_device(addr).await {
                println!("Could not remove cached device {}: {}", addr, e);
            }
        }
    }

    // LE only. Macs are dual-mode: with Auto (interleaved BR/EDR inquiry + LE scan), a
    // discoverable Mac (e.g. Bluetooth Settings pane open) collapses into one device entry
    // whose Connect() can pick the BR/EDR bearer. That connection pairs over SSP, resolves
    // services via SDP, and leaves the GATT database empty — the "Did not read all Flying
    // Carpet characteristics" failure. All Flying Carpet peers advertise over LE.
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
        while let Some(evt) = discover.next().await {
            match evt {
                AdapterEvent::DeviceAdded(addr) => {
                    let device = adapter.device(addr)?;
                    // The pre-seeded events include unrelated known devices (the discovery
                    // filter does not apply to them); only accept our service.
                    let dev_uuids = device.uuids().await.ok().flatten().unwrap_or_default();
                    if !dev_uuids.contains(&fc_uuid) {
                        println!("Ignoring device {} without Flying Carpet service", addr);
                        continue;
                    }
                    return Ok(device);
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
    let peer = Peer::try_from(peer_os.as_str()).map_err(|e| bluer::Error {
        kind: ErrorKind::ServicesUnresolved,
        message: e.to_string(),
    })?;
    if is_hosting(&peer, mode) {
        // write ssid and password
        let password = generate_password();
        let (_, ssid) = get_key_and_ssid(&password);
        ssid_char.write_ext(ssid.as_bytes(), &write_req).await?;
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
