# FlyingCarpet Connection Architecture

## Overview

A transfer involves four independent role axes. Each device takes one role on each axis. The roles are determined by a combination of **which device is sending/receiving** and **the peer's operating system**.

| Axis | Role A | Role B |
|------|--------|--------|
| **Transfer direction** | Sender | Receiver |
| **BLE** | Peripheral (advertiser) | Central (scanner) |
| **WiFi hotspot** | Host | Joiner |
| **TCP** | Server (listener) | Client (connector) |

These four axes are **not all aligned** — the mapping depends on the platform pair and connection mode.

---

## Bluetooth Roles

BLE role is determined **solely by transfer direction**, the same on every platform:

| Transfer Direction | BLE Role |
|-------------------|----------|
| **Sender** | **Peripheral** — advertises GATT service, exposes characteristics |
| **Receiver** | **Central** — scans for peripheral, connects, reads/writes characteristics |

Source:
- Linux: `core/src/linux/bluetooth.rs` — `Mode::Send` → peripheral branch (line 85), `else` → central branch (line 156)
- Windows: `core/src/windows/bluetooth.rs` — `Mode::Send` → peripheral branch (line 74), `else` → central branch (line 144)
- Android: `MainActivity.kt` — `Mode.Sending` → `advertise()`, `Mode.Receiving` → `scan()` (lines 83-88)

### BLE Data Flow

The GATT service exposes three characteristics: **OS**, **SSID**, **Password**.

Data flow direction depends on who is **hosting the hotspot**, not who is the BLE peripheral/central:

| Hosting device is... | SSID/Password flow |
|----------------------|-------------------|
| **Peripheral** (sender is hosting) | Peripheral populates GATT; central reads |
| **Central** (receiver is hosting) | Central writes SSID/password to peripheral's GATT |

The OS characteristic always flows both ways: each side needs to know the other's OS.

---

## Hotspot Hosting

The **hotspot host generates the WiFi password**, creates the hotspot, and acts as **TCP server**. The **joiner** connects to the hotspot and acts as **TCP client**.

### Linux `is_hosting(peer, mode)`

| Peer OS | Linux Hosts? |
|---------|-------------|
| Android | **Always** |
| iOS     | **Always** |
| macOS   | **Always** |
| Windows | **Never** |
| Linux   | **Only if Receiving** |

Source: `core/src/linux/network.rs` lines 11-20

### Windows `is_hosting(peer, mode)`

| Peer OS | Windows Hosts? |
|---------|---------------|
| Android | **Always** |
| iOS     | **Always** |
| macOS   | **Always** |
| Linux   | **Always** |
| Windows | **Only if Receiving** |

Source: `core/src/windows/network.rs` lines 674-683

### Android `isHosting()`

| Peer OS | Android Hosts? |
|---------|---------------|
| iOS     | **Always** |
| macOS   | **Always** |
| Android | **Only if Receiving** |
| Linux   | **Never** (Linux hosts) |
| Windows | **Never** (Windows hosts) |

Source: `MainViewModel.kt` lines 112-116

### iOS / macOS

iOS and macOS **never host hotspots** — they lack a public hotspot API. The peer always hosts.

---

## TCP Roles (Hotspot Mode)

TCP role follows directly from hotspot role:

| Hotspot Role | TCP Role |
|-------------|----------|
| **Host** | **Server** — binds `0.0.0.0:3290`, calls `accept()` |
| **Joiner** | **Client** — connects to host's gateway IP on port 3290 |

Source:
- Rust: `core/src/lib.rs` `start_tcp()` — `PeerResource::WifiClient` → connect, otherwise → bind+accept
- Android: `MainViewModel.kt` `startTCP()` — `isHosting()` → `ServerSocket(3290).accept()`, else → `Socket(peerIP, 3290)`

---

## Password Generation & Sharing

**The hotspot host generates the password.** It is shared with the peer via one of:

1. **Bluetooth** — delivered through GATT characteristics (direction depends on who's hosting, see BLE Data Flow above)
2. **QR code** — host displays QR code, joiner scans it (for mobile peers)
3. **Manual entry** — host displays password as text, joiner types it in

The desktop app's `needPassword()` function:
- Returns **false** → this device is **hosting** → generates and displays the password
- Returns **true** → this device is **joining** → user must enter the host's password

---

## Complete Platform Pair Matrix (Hotspot Mode)

"A → B" means A is sending to B.

| Scenario | BLE: Peripheral | BLE: Central | Hotspot Host | TCP Server | Password Generator |
|----------|----------------|-------------|-------------|------------|-------------------|
| Linux → Android | Linux | Android | Linux | Linux | Linux |
| Android → Linux | Android | Linux | Linux | Linux | Linux |
| Linux → Windows | Linux | Windows | Windows | Windows | Windows |
| Windows → Linux | Windows | Linux | Windows | Windows | Windows |
| Linux → iOS/macOS | Linux | iOS/macOS | Linux | Linux | Linux |
| Windows → iOS/macOS | Windows | iOS/macOS | Windows | Windows | Windows |
| Android → iOS/macOS | Android | iOS/macOS | Android | Android | Android |
| Linux → Linux | Sender | Receiver | Receiver | Receiver | Receiver |
| Windows → Windows | Sender | Receiver | Receiver | Receiver | Receiver |
| Android → Android | Sender | Receiver | Receiver | Receiver | Receiver |
| Windows → Android | Windows | Android | Windows | Windows | Windows |
| Android → Windows | Android | Windows | Windows | Windows | Windows |

**Key pattern: Hotspot Host = TCP Server = Password Generator.** BLE Peripheral = Sender, BLE Central = Receiver. These two groupings are independent.

In shared network mode, there is no hotspot host. The **Receiver** takes over as TCP Server and Password Generator — consistent with the same-platform hotspot convention. See the Shared Network Mode section below.

---

## Shared Network Mode

No hotspot is created. Both devices are already on the same LAN.

### Discovery

Both devices simultaneously:
1. Send HMAC-signed announcements via UDP multicast (`239.255.73.67:3290`) and unicast subnet scan
2. Listen for announcements from the peer
3. Validate: magic bytes, HMAC (using password-derived key), timestamp window, opposite role

### TCP in Shared Network Mode

| Transfer Direction | TCP Role |
|-------------------|----------|
| **Receiver** | **Server** — binds TCP listener on port 3290 *before* discovery starts |
| **Sender** | **Client** — connects to receiver's IP after discovery completes |

This is consistent with same-platform hotspot mode, where the receiver always hosts and is the TCP server.

### Password in Shared Network Mode (without Bluetooth)

- **Receiver** generates the password and displays it
- **Sender** enters the password manually

Consistent with hotspot mode: the receiver is always the "anchor" role (host in hotspot, server + password generator in shared network).

The password derives both the HMAC key for discovery authentication and the AES key for file encryption.

### Bluetooth + Shared Network Mode

The current BLE protocol is tightly coupled to the hotspot flow — it exchanges peer OS, SSID, and password, and uses `is_hosting()` to determine BLE data flow direction. In shared network mode, peer OS and SSID are irrelevant, and `is_hosting()` doesn't apply.

**Status: Not yet supported.** Adapting the BLE protocol for shared network mode (exchanging only the password, with a fixed data flow convention like sender-peripheral-provides / receiver-central-reads) is a future enhancement.
