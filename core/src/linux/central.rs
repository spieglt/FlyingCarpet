//! Bluetooth central (receiving) side of the transfer negotiation.
//!
//! # History: the macOS -> Linux Bluetooth failure
//!
//! Sending from macOS to Linux used to require manually pairing the two machines from the
//! macOS side first. The root cause: when macOS acts as the BLE peripheral (Flying
//! Carpet's sending side), it advertises with its public Bluetooth address and with
//! advertisement flags declaring simultaneous LE and BR/EDR (classic Bluetooth) support —
//! CoreBluetooth provides no way to change either. BlueZ counts every such advertisement
//! as a sighting of *both* bearers (adapter.c update_found_devices), and when connecting
//! to an unbonded dual-mode device, Device1.Connect() breaks the "most recently seen
//! bearer" tie in favor of classic (device.c select_conn_bearer, verified in 5.72). So
//! Linux always connected to the Mac over classic Bluetooth — which serves none of the
//! app's GATT services — and the transfer failed to find the Flying Carpet
//! characteristics. Explicit Device1.Pair() selects its bearer the same way, so pairing
//! through BlueZ's API couldn't fix it (and a classic pairing makes it worse: SSP + CTKD
//! leave a dual-transport bond, which still loses the tiebreak). iOS, Windows, and
//! Android peripherals never hit any of this because they advertise with random
//! addresses, which BlueZ only ever connects over LE. Manually pairing from the macOS
//! side only "worked" by leaving behind bond state that happened to steer BlueZ to LE.
//!
//! The one rule that overrides the tiebreak: BlueZ prefers the bonded bearer when exactly
//! one bearer is bonded. So the fix (in find_characteristics below) is to create an
//! LE-only bond ourselves before ever calling Connect(): open an LE L2CAP socket that
//! requires high security, which makes the kernel run numeric-comparison SMP pairing
//! (confirmation code on both devices, answered on ours by the agent registered in
//! bluetooth.rs) on the LE link. Every later Connect() then dials LE.

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
use tokio::time::{interval, sleep, timeout};

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
    Mode, Peer, UI,
};

pub async fn find_characteristics<T: UI>(
    device: &Device,
    ui: &T,
) -> Result<HashMap<&'static str, Characteristic>> {
    let addr = device.address();
    let uuids = device.uuids().await?.unwrap_or_default();

    let os_characteristic_uuid = Uuid::parse_str(OS_CHARACTERISTIC_UUID).unwrap();
    let ssid_characteristic_uuid = Uuid::parse_str(SSID_CHARACTERISTIC_UUID).unwrap();
    let password_characteristic_uuid = Uuid::parse_str(PASSWORD_CHARACTERISTIC_UUID).unwrap();
    println!("Discovered device {} with service UUIDs {:?}", addr, &uuids);
    let md = device.manufacturer_data().await?;
    println!("    Manufacturer data: {:x?}", &md);

    // The cached UUIDs property is stale for bonded peers (see scan()), so a device that
    // scan() already verified over a live connection would be rejected here on the same bad
    // evidence. If it's connected, scan() vouched for it — the service enumeration below is
    // the real check either way.
    let already_connected = device.is_connected().await.unwrap_or(false);
    if uuids.contains(&Uuid::parse_str(SERVICE_UUID).unwrap()) || already_connected {
        println!("    Device provides our service!");
        ui.output("Peer is running Flying Carpet, connecting over Bluetooth...");
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
            ui.output("Bonding over Bluetooth LE...");
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
            ui.output("Bluetooth bond established");
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
            ui.output("Connected to peer over Bluetooth");
        } else {
            println!("    Already connected");
            ui.output("Already connected to peer over Bluetooth");
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
                    ui.output("Found Flying Carpet's Bluetooth service on peer");
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
            ui.output("Waiting for peer's Bluetooth characteristics, retrying...");
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

// Connect and ask BlueZ to resolve the peer's GATT database, then report whether our service
// is actually there. Used when the cached UUIDs property can't be trusted (bonded peers).
async fn probe_for_service(device: &Device, fc_uuid: &Uuid) -> bluer::Result<bool> {
    if !device.is_connected().await? {
        device.connect().await?;
    }
    // services() waits for ServicesResolved internally, with bluer's own timeout
    let services = device.services().await?;
    let mut found = false;
    for service in &services {
        if service.uuid().await? == *fc_uuid {
            found = true;
        }
    }
    println!(
        "    resolved {} services on {}; Flying Carpet present: {}",
        services.len(),
        device.address(),
        found
    );
    Ok(found)
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
        // discover_devices_with_changes, NOT discover_devices. The plain version emits
        // DeviceAdded once per device, and for a bonded peer that once is the pre-seeded cache
        // entry: BlueZ resolves the peer's rotating private address back to the existing entry
        // using the stored IRK, so no amount of live advertising produces a second DeviceAdded.
        // If that cached entry predates the peer ever advertising our service — it was created
        // when the peer connected to *us* as a central, so it records the peer's GATT server
        // rather than the service it advertises when sending — the filter below dismisses it
        // and the scan waits forever on an event that can never arrive. That is the
        // Windows -> Linux hang.
        //
        // The _with_changes variant subscribes to each device's property stream and re-emits
        // DeviceAdded on every change, so the peer is picked up the moment BlueZ merges the
        // service UUID in from a live advertisement. Purging the cached entry the way the
        // unpaired ones above are purged is not an option: remove_device takes the bond too.
        let discover = adapter.discover_devices_with_changes().await?;
        pin_mut!(discover);
        // DeviceAdded now repeats per property change, so only log/probe each address once.
        let mut reported: HashSet<Address> = HashSet::new();
        let mut probed: HashSet<Address> = HashSet::new();
        let mut diag = interval(Duration::from_secs(5));
        diag.tick().await; // the first tick is immediate; we want the first dump at +5s
        loop {
            tokio::select! {
                event = discover.next() => {
                    let Some(event) = event else { break };
                    match event {
                        AdapterEvent::DeviceAdded(addr) => {
                            let device = adapter.device(addr)?;
                            // Known devices are included regardless of the discovery filter,
                            // so check the service ourselves.
                            let dev_uuids = device.uuids().await.ok().flatten().unwrap_or_default();
                            if dev_uuids.contains(&fc_uuid) {
                                println!("Found peer {}", addr);
                                return Ok(device);
                            }
                            // For a *bonded* device, BlueZ's UUIDs property is its cached view
                            // of the peer's GATT database from the last connection, and it is
                            // NOT refreshed from advertisements. Observed 2026-07-25: the peer
                            // sat at rssi -50 listing only its generic services (1800, 1801,
                            // 180a, 1849, 184c, 1855) for as long as we scanned, while it was
                            // advertising Flying Carpet the whole time. Unbonded peers never
                            // hit this — with no cache, the advertised UUID is all BlueZ has,
                            // which is why first-time pairing always worked and every reuse
                            // hung. Purging the entry to force rediscovery is not an option:
                            // remove_device takes the bond with it.
                            //
                            // So ask over an actual connection instead. device.services()
                            // waits for BlueZ to resolve the GATT database, which is the
                            // source of truth the cached property is only an approximation of.
                            if device.is_paired().await.unwrap_or(false) && probed.insert(addr) {
                                println!(
                                    "Paired device {} doesn't list our service; connecting to re-resolve its GATT database",
                                    addr
                                );
                                match probe_for_service(&device, &fc_uuid).await {
                                    Ok(true) => {
                                        println!("Found peer {} after re-resolving services", addr);
                                        return Ok(device);
                                    }
                                    Ok(false) => {
                                        println!("    {} has no Flying Carpet service; disconnecting", addr);
                                        let _ = device.disconnect().await;
                                    }
                                    Err(e) => {
                                        println!("    Could not probe {}: {}", addr, e);
                                        let _ = device.disconnect().await;
                                    }
                                }
                            }
                            if reported.insert(addr) {
                                println!(
                                    "Device {} has no Flying Carpet service yet (paired: {:?}, connected: {:?}, rssi: {:?})",
                                    addr,
                                    device.is_paired().await,
                                    device.is_connected().await,
                                    device.rssi().await.ok().flatten(),
                                );
                            }
                            continue;
                        }
                        AdapterEvent::DeviceRemoved(addr) => {
                            reported.remove(&addr);
                            println!("Device removed {addr}");
                        }
                        other_event => println!("Processed other event: {:?}", other_event),
                    }
                }
                // Periodic dump of what BlueZ believes, so a scan that finds nothing says why
                // rather than going silent. In particular this shows whether a bonded peer's
                // UUID list ever gains our service while it is advertising.
                _ = diag.tick() => {
                    println!("--- still scanning; known devices ---");
                    for addr in adapter.device_addresses().await? {
                        let device = adapter.device(addr)?;
                        println!(
                            "    {} name {:?} paired {:?} connected {:?} rssi {:?} uuids {:?}",
                            addr,
                            device.name().await.ok().flatten(),
                            device.is_paired().await,
                            device.is_connected().await,
                            device.rssi().await.ok().flatten(),
                            device.uuids().await.ok().flatten(),
                        );
                    }
                }
            }
        }
        println!("Stopping discovery");
    }
    Err(bluer::Error {
        kind: ErrorKind::NotFound,
        message: "Exited scan() without finding device".to_string(),
    })
}

// Every step here is mirrored to the UI as well as stdout. The peripheral path reports the
// same events through process_bluetooth_message, so previously a Linux user acting as
// central saw the app sit silent for several seconds while all of this happened — the
// wording matches process_bluetooth_message and the Windows implementation.
pub async fn exchange_info<T: UI>(
    characteristics: HashMap<&str, Characteristic>,
    mode: &Mode,
    ui: &T,
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
    ui.output(&format!("Peer's OS is {}", peer_os));
    sleep(Duration::from_secs(1)).await;
    // write our OS
    os_char.write_ext(OS.as_bytes(), &write_req).await?;
    println!("Wrote OS to peer");
    ui.output("Wrote our OS to peer");
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
        ui.output("Wrote our SSID to peer");
        sleep(Duration::from_secs(1)).await;
        password_char
            .write_ext(password.as_bytes(), &write_req)
            .await?;
        println!("Wrote password to peer");
        ui.output("Wrote our password to peer");
        sleep(Duration::from_secs(1)).await;
        Ok((peer_os, ssid, password))
    } else {
        // read ssid and password
        let ssid = ssid_char.read().await?;
        let ssid = String::from_utf8(ssid).expect("SSID was not UTF-8");
        println!("Peer's SSID: {}", ssid);
        ui.output(&format!("Peer's SSID is {}", ssid));
        let password = password_char.read().await?;
        let password = String::from_utf8(password).expect("Password was not UTF-8");
        println!("Peer's password: {}", password);
        ui.output(&format!("Peer's password is {}", password));
        Ok((peer_os, ssid, password))
    }
}
