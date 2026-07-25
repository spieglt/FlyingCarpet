# Bluetooth field guide

Hard-won knowledge from the 2026-07-25 Windows↔Linux debugging session, plus a cross-platform
audit of where each of the five platforms stands. **Read this before touching any BLE code.**

Companion docs: `docs/windows-ble-gatt-0x8000ffff.md` (the running investigation log, in
chronological order) and `docs/ble-bond-asymmetries.md` (the audit that predicted several of
these). `ARCHITECTURE.md` covers why Bluetooth is hotspot-only.

---

## 1. The mental model

Flying Carpet's BLE usage has **four independent axes**. Almost every bug in this session came
from conflating two of them, or from assuming a platform behaves the same on one axis as it
does on another.

| Axis | Question | Who controls it |
|---|---|---|
| **Advertising** | can anyone *discover* me? | the peripheral (sender) |
| **Scanning** | how do I recognize the peer? | the central (receiver) |
| **Bonding** | do we have shared keys, and *over which transport*? | both, at pairing time |
| **GATT service** | is the service registered, and what does the peer's cache think? | the peripheral registers; the central caches |

The roles reverse between transfers by design — **sender = BLE peripheral, receiver = BLE
central** — so every device plays both sides, and state created in one role is read back in
the other. That is the source of the whole bug class.

### The two axes that actually bite

**Bearer selection (LE vs. BR/EDR).** A dual-mode peer can be reached over classic Bluetooth
or over LE. Flying Carpet's GATT service exists **only over LE**. Any stack that picks the
bearer for you can pick wrong, and the failure looks like "connected but no services".

**GATT caching.** Bonding exists partly so the central can skip rediscovery. That means the
central holds a snapshot of the peer's service database — and Flying Carpet's peripherals
*remove* their service when a transfer ends, so that snapshot goes stale by design.

---

## 2. The laws

Learned the hard way. Violate these and you get an intermittent hang weeks later.

1. **Never let the stack choose the bearer.** Always demand LE explicitly. Linux needs an
   L2CAP LE socket to force it; Android needs `TRANSPORT_LE`, never `TRANSPORT_AUTO`.

2. **A bond's *provenance* determines its transport.** A bond created while you were the
   central (you dialed LE) is LE-only. A bond created while you were the peripheral (the peer
   dialed in) is **dual-transport**, because cross-transport key derivation mints BR/EDR keys
   from the LE pairing. Same peer, same bond record, opposite behavior on the next connect.

3. **Filter on advertisement data, never on resolved-service data.** BlueZ's `Device1.UUIDs`
   is "the available remote services" — a cached GATT snapshot — while `ServiceData` is the
   advertisement. They are different sources with confusingly similar names. Windows, Android
   and Apple all filter on advertisement data and were never vulnerable; Linux filtered on the
   cache and hung on every bonded peer.

4. **Never unpair unilaterally.** You cannot tell the peer, and Apple platforms cannot clear
   their half programmatically at all. A one-sided bond is *worse* than a bad bond: the peer
   still believes it can skip pairing, and encrypts with a key you threw away. Symptoms are
   CBError 14 on macOS and an empty GATT service list on Windows.

5. **Bonded and unbonded are different code paths on every stack.** Every platform's discovery
   logic was written and tested against the unbonded path, because that's what a first run
   exercises. The bonded path is what users actually hit from the second transfer onward.

6. **Read the error string's prefix.** `br-connection-canceled` names its own cause. Three
   commits were built on the wrong theory with that string already in the log.

7. **Check the Linux Bluetooth history before theorizing.** `967ed6b` had worked out the
   bearer tiebreak, CTKD dual bonds, and the L2CAP-socket cure a month before this session
   re-derived them from scratch against a different peer.

---

## 3. What actually happened this session

Four distinct bugs, each masking the next. Symptoms in order of appearance:

### Bug 1 — Linux deleted the peer's bond after every transfer

**Symptom:** Windows→Linux, then Linux→Windows: Windows reports `AlreadyPaired`, then
`GetGattServicesWithCacheModeAsync` returns **success with an empty service list**. Retries
don't help. The recovery ladder unpairs, re-pairs, and then fails permanently.

**Cause:** `keep_bond = info.0 == "mac"` — Linux removed the bond for any non-macOS peer, on
the premise that "Windows and Android re-pair per transfer." Both halves of that premise were
false: Windows short-circuits on `AlreadyPaired`, and Android has no `removeBond` call
anywhere. Windows kept its half and encrypted with an LTK Linux had discarded; the link never
encrypted, so there was no GATT database, which WinRT reports as success-with-nothing.

**Fix:** `6039d53` — never remove bonds on cleanup. macOS was never special; it was the only
platform whose error message (CBError 14, "Peer removed pairing information") named the
problem out loud.

### Bug 2 — Windows conflated "no service" with "no connection"

**Symptom:** the misleading message above — "Flying Carpet service not found in peer's service
list" — which sent the recovery ladder after the peer's GATT database instead of the link.

**Cause:** `get_services_and_characteristics` called `.Services()` without checking
`GattDeviceServicesResult::Status()`. On anything but `Success` the collection is empty, which
is indistinguishable from "connected fine, service genuinely absent."

**Fix:** `6039d53` — check `Status()` first, report an empty-but-successful list as its own
condition, and name a one-sided bond as the likely cause in both messages.

### Bug 3 — Linux's scan filtered on a property that can never change

**Symptom:** Windows advertises; Linux sits at "Started Bluetooth scan, waiting for sending
device..." forever. The diagnostic dump shows the peer present at rssi −50, bonded, with a
UUID list that never gains the Flying Carpet service.

**Cause, two layers.** First, `AdapterEvent::DeviceAdded` fires only *once* per device, and
for a bonded peer that once is the pre-seeded cache entry — BlueZ resolves the peer's rotating
private address back to the existing record via the stored IRK. Second and more fundamental,
the property being checked (`Device1.UUIDs`) is the resolved-GATT-services list, not
advertisement data, so it reflects a cached snapshot taken when the peer had no Flying Carpet
service registered.

**Fixes:** `573fb87` (use `discover_devices_with_changes`, which re-emits on property change)
and `6270ac4` (when a bonded peer's cached UUIDs lack the service, connect and ask for the
resolved database instead of trusting the cache).

### Bug 4 — the one that mattered: `Connect()` was dialing classic Bluetooth

**Symptom:** the probe from bug 3 failing with
`Could not probe E8:48:…: Bluetooth operation failed: br-connection-canceled`.

**Cause:** `br-` is BR/EDR. BlueZ's `select_conn_bearer` prefers the bonded bearer when
exactly one is bonded, otherwise the most recently seen, with BR/EDR winning ties. A bond
created while Linux was the peripheral is dual-transport (CTKD), so both bearers are bonded,
the tiebreak falls through, and classic wins — against a peer that serves GATT only over LE.

The existing LE-bonding socket was gated on `address_type() == LePublic`, with the comment
"random-address peers (Windows/Android/iOS) always connect over LE anyway." Windows advertises
from `E8:48:…` (`0xE8` = `0b11101000`, a **static random** address), so it was excluded. That
assumption is true only while a peer is *unbonded*.

**Fix:** `386f654` — `ensure_le_link()` raises the LE ACL link with an L2CAP LE socket before
`Connect()` for any already-paired peer, regardless of address type. **This is the fix that
made it work.**

### Why the ordering confused everything

The two bond provenances produce opposite outcomes, so the same pair of machines passed or
hung depending on which direction you transferred *first*:

| First transfer | Linux's bond with the peer | Second transfer |
|---|---|---|
| Windows→Linux (Linux central) | LE-only, made by the L2CAP socket | works |
| Linux→Windows (Linux peripheral) | dual-transport, made by CTKD | `br-connection-canceled` |

---

## 4. Where all five platforms stand

Verified by reading the code on 2026-07-25. ✅ correct · ⚠️ works but fragile · ❌ known gap.

### Advertising (peripheral / sender)

| | Mechanism | Stops between transfers | Address type |
|---|---|---|---|
| **Windows** | `GattServiceProvider.StartAdvertising` | ✅ explicit `stop_advertising()` (`012d00b`) | static random |
| **Linux** | bluer `Advertisement` | ✅ `drop(adv_handle)` after the OS exchange | adapter (usually public) |
| **Android** | `bluetoothLeAdvertiser` | ✅ `stopAdvertising` + GATT server closed | random |
| **iOS** | `CBPeripheralManager` | ✅ `stopAdvertising` + `removeService` | random |
| **macOS** | `CBPeripheralManager` | ✅ same | ⚠️ **public + dual-mode flags**, unavoidable — the original macOS↔Linux bug |

### Scanning (central / receiver) — what the peer is matched on

| | Filter source | Verdict |
|---|---|---|
| **Windows** | `advertisement.ServiceUuids()` | ✅ advertisement data |
| **Android** | `ScanFilter.setServiceUuid` | ✅ advertisement data |
| **iOS/macOS** | `scanForPeripherals(withServices:)` | ✅ advertisement data |
| **Linux** | `DiscoveryFilter` + `Device1.UUIDs` | ⚠️ fixed, but `UUIDs` is *resolved services*; the probe path now compensates |

### Bearer selection — the axis that caused the outage

| | Choice | Verdict |
|---|---|---|
| **Windows** | n/a — `BluetoothLEDevice` is LE by definition | ✅ immune |
| **iOS/macOS** | n/a — CoreBluetooth is LE-only | ✅ immune |
| **Linux** | BlueZ `select_conn_bearer` | ✅ forced LE via L2CAP socket, unbonded *and* bonded |
| **Android** | `TRANSPORT_LE` on the scan path… | ❌ **`TRANSPORT_AUTO` on the post-bond path** — see §5 |

### Bond retention

| | After a successful transfer | On failure recovery |
|---|---|---|
| **Windows** | ✅ keeps (unpairing deliberately disabled) | ⚠️ ladder rung 2 unpairs unilaterally |
| **Linux** | ✅ keeps (`6039d53`) | ⚠️ poisoned-bond self-heal calls `remove_device` |
| **Android** | ✅ keeps (no `removeBond` anywhere) | ✅ none |
| **iOS/macOS** | ✅ keeps (no API to remove) | ✅ none possible |

All five now agree on the happy path. The two ⚠️s violate law 4 and are the remaining hazard.

### GATT service registration and cache invalidation

| | Service lifetime | Central-side cache handling |
|---|---|---|
| **Windows** | registered per transfer | ✅ `Uncached` + `Status()` checked — the only fully correct one |
| **Linux** | per transfer (`drop(app_handle)`) | ⚠️ connects to re-resolve when cached UUIDs look wrong |
| **Android** | per transfer (server closed) | ❌ `onServiceChanged` → `discoverServices()` **commented out** |
| **iOS** | per transfer (`removeService`) | ❌ `didModifyServices` logs only |
| **macOS** | per transfer | ❌ `didModifyServices` not implemented at all |

---

## 5. Outstanding, ranked

### 1. Android uses `TRANSPORT_AUTO` immediately after bonding — same bug, same class

`Bluetooth.kt`, the `ACTION_BOND_STATE_CHANGED` receiver:

```kotlin
result!!.device.connectGatt(
    application.applicationContext,
    true,
    gattCallback,
    BluetoothDevice.TRANSPORT_AUTO,   // <-- lets Android pick the bearer
)
```

The scan path five hundred lines earlier correctly passes `TRANSPORT_LE`. This one fires the
moment bonding completes — precisely when CTKD has just created a dual-transport bond, which
is the exact condition that made BlueZ pick BR/EDR. Against a dual-mode peer (any desktop, and
macOS especially) Android can connect over classic, which serves no GATT.

**Fix: change it to `TRANSPORT_LE`.** One token. This is the highest-value outstanding item in
this document, and it is the direct analogue of the bug that cost this session.

The unchecked Android↔Windows and Android↔Linux hotspot rows in the test plan are exactly the
ones that would expose it.

### 2. Android fails silently when the service or characteristics are missing

Four exits from `onServicesDiscovered` — three printing nothing, none calling
`bluetoothFailed()`, no timeout behind them. Linux errors, Windows retries then errors, Apple
calls `cleanUpTransfer()`; Android hangs. Any of the bugs above, hit on Android, presents as
"it just hangs" with nothing in the log. Fix regardless of whether item 1 is real.

### 3. The two unilateral-unpair paths violate law 4

Windows' recovery ladder rung 2 and Linux's poisoned-bond self-heal both destroy a bond the
peer keeps. That's bug 1, re-created on demand. Now that bug 4 is fixed they should fire far
less — but when they do, they should at minimum tell the user to remove the pairing on the
*other* device too, since that's the only way out and Apple peers can't be fixed any other
way.

### 4. Android and Apple never invalidate their GATT cache

`onServiceChanged` commented out; `didModifyServices` a no-op on iOS and absent on macOS.
Every peripheral removes its service at teardown, so a bonded central can hold a stale
snapshot. Apple degrades to a clean error (the `didUpdateValueFor` error path added recently);
Android degrades to item 2's silent hang.

### 5. macOS `didModifyServices` missing entirely

Two targets sharing `Bluetooth.swift` that differ on a delegate method. Implement it to match
iOS even if it only logs.

---

## 6. Debugging playbook

| Symptom | Most likely cause | First thing to check |
|---|---|---|
| Connected, but **zero** services | link never encrypted — one-sided bond | is the peer still bonded? `bluetoothctl paired-devices` |
| Connected, services present, **ours missing** | stale GATT cache, or peer hadn't registered it yet | did the peer register the service *before* you connected? |
| Error containing **`br-`** | classic bearer chosen for an LE-only service | force LE; check bond provenance |
| Scan never finds an advertising peer | filtering on cached rather than advertised data | does the filter read advertisement data? |
| Works on first pairing, fails on reuse | bonded-vs-unbonded divergence | test both bond provenances (see below) |
| macOS: **CBError 14** | peer deleted its half of the bond | law 4 |
| Windows: `0x8000FFFF` on enumeration | unresolved; retry ladder | `docs/windows-ble-gatt-0x8000ffff.md` |

**Always test both bond provenances.** From fully unpaired, run A→B then B→A; then unpair
both sides and run B→A then A→B. These are *different code paths* and only one of them was
passing for the entire v10 cycle.

**Useful instruments:** `sudo btmon` (raw HCI — the only way to see which bearer and what's
actually in the advertisement), `bluetoothctl info <addr>` (what BlueZ believes),
`adb logcat -s FlyingCarpet` (Android), `cargo tauri dev` stdout on both desktops. UI logs
alone were insufficient for every bug in this session.
