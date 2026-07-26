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

8. **"Connected" is not a bearer, and it is not a GATT database.** BlueZ's `Device1.Connected`
   is `bredr_state.connected || le_state.connected` — a peer reachable only over classic reads
   as connected. Never let it stand in for "we have a usable LE link"; `ServicesResolved` is
   the property that means that. Every guard of the form `if !is_connected() { force LE }` is
   this bug waiting to happen, because it skips the fix in the one case that needs it.

9. **Hang up when you're done.** A link left open outlives the transfer and is inherited by the
   next one *in the opposite role*, where it silently satisfies every "are we connected?" check
   while being the wrong bearer, the wrong direction, or attached to a service that has since
   been removed. Dropping the GATT service without dropping the link also strands the peer's
   cache: it holds a snapshot of a database that no longer exists, with no Service Changed
   coming. Disconnect ≠ unpair — law 4 is about the *bond*, and this is about the *link*.

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

## 3a. The Android sequel — same axis, one layer further in (2026-07-25)

`386f654` fixed `Connect()` picking BR/EDR. It did not fix the case where **something else
already established the link**, which is what Linux↔Android surfaced.

**Symptom:** Linux→Android (pairing along the way), then Android→Linux. Android advertises
happily. Linux logs, twice:

```
Found device
Peer is running Flying Carpet, connecting over Bluetooth...
Already connected to peer over Bluetooth
Bluetooth connection failed; retrying...
```

and never exchanges credentials.

**Cause, three defects stacked in the order they fire:**

1. **Linux never disconnected, in either role.** `negotiate_bluetooth` dropped the
   advertisement and the GATT application at the end of a transfer but left the ACL up. There
   was no `disconnect()` anywhere outside `scan()`'s probe-failure path. So the reverse
   transfer began with a live link left over from the previous one.

2. **`Device1.Connected` is bearer-agnostic (law 8), so the leftover link satisfied every
   check.** `find_characteristics` took its "already connected" arm, which skipped `Connect()`
   *and* `ensure_le_link()`. The bond in this direction is the dual-transport CTKD kind — the
   previous transfer had Linux as the peripheral — so the inherited link can be the bearer that
   serves no GATT.

3. **`ensure_le_link()` would have been a no-op anyway.** Its first statement was
   `if is_connected() { return }` — the exact property that cannot distinguish the two bearers.
   The function written to force LE disabled itself in precisely the situation it existed for.

Then `device.services()` sat in bluer's `wait_for_services_resolved` (120 s `TIMEOUT`) and
returned `ServicesUnresolved`. The retry rung re-ran an identical attempt, because nothing had
torn the link down — hence the log repeating verbatim.

**Fixes:** `ensure_le_link()` short-circuits on `ServicesResolved` instead of `Connected` and is
now called for every paired peer regardless of connection state (`central.rs`); the retry rung
disconnects first; and both roles hang up when the exchange is done (`bluetooth.rs`).

This was already recorded as a symptom in a `lib.rs` TODO — *"linux can't receive from windows
or android if already paired/connected, service not found. but then it disconnects and next
transfer works"* — including the cure. Law 7 again.

### Why Android makes this easy to hit

Android is the only platform whose Flying Carpet GATT service is registered **permanently**,
not per transfer. `Bluetooth.stop()` calls `initializePeripheral()`, which closes the old server
and immediately opens a new one *and re-adds the service*; `MainActivity` does the same whenever
Bluetooth is switched on. So BlueZ's cached `Device1.UUIDs` for an Android peer always contains
our service, and `scan()` returns it from cache instantly — without waiting for a live
advertisement, and without ever taking the bonded-peer probe path that would have re-resolved
the database. Fast, and wrong-bearer failures surface immediately rather than after a scan.

---

## 4. Where all five platforms stand

Verified by reading the code on 2026-07-25. ✅ correct · ⚠️ works but fragile · ❌ known gap.

### Advertising (peripheral / sender)

| | Mechanism | Stops between transfers | Address type |
|---|---|---|---|
| **Windows** | `GattServiceProvider.StartAdvertising` | ✅ explicit `stop_advertising()` (`012d00b`), plus a `Drop` guard for error/cancel paths — WinRT holds its own reference, so dropping ours is not documented to stop it | static random |
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
| **Linux** | BlueZ `select_conn_bearer` | ✅ forced LE via L2CAP socket, unbonded *and* bonded, and now on inherited links too (§3a) |
| **Android** | `TRANSPORT_LE` on both paths | ✅ post-bond path fixed in `6b29695` — see §5.1 |

### Bond retention

| | After a successful transfer | On failure recovery |
|---|---|---|
| **Windows** | ✅ keeps (unpairing deliberately disabled) | ✅ one unpair, last resort, warns the user |
| **Linux** | ✅ keeps (`6039d53`) | ✅ retry first; `remove_device` last resort, warns the user |
| **Android** | ✅ keeps (no `removeBond` anywhere) | ✅ none |
| **iOS/macOS** | ✅ keeps (no API to remove) | ✅ none possible |

All five agree on the happy path, and the two failure paths that could still strand a peer now
try everything else first and tell the user what to do on the other device.

### GATT service registration and cache invalidation

| | Service lifetime | Central-side cache handling |
|---|---|---|
| **Windows** | registered per transfer | ✅ `Uncached` + `Status()` checked |
| **Linux** | per transfer (`drop(app_handle)`) | ✅ connects to re-resolve when cached UUIDs look wrong |
| **Android** | ⚠️ **permanent** — `stop()` calls `initializePeripheral()`, which reopens the server *and re-adds the service* | ✅ `onServiceChanged` → `discoverServices()`, gated on `exchangeComplete` |
| **iOS** | per transfer (`removeService`) | ✅ shared `didModifyServices` re-discovers (`FlyingCarpetApple` `4c59af6`, pending an Xcode build) |
| **macOS** | per transfer | ✅ same shared helper, same caveat |

Every stack that does re-discover depends on the peer sending a Service Changed indication.
Where one isn't sent, no stack learns its cache is stale — see §5.4.

---

## 5. Outstanding, ranked

### 1. ~~Android uses `TRANSPORT_AUTO` immediately after bonding~~ — fixed in `6b29695`

`Bluetooth.kt`'s `ACTION_BOND_STATE_CHANGED` receiver passed `TRANSPORT_AUTO` to the
post-bond `connectGatt` while the scan path five hundred lines earlier correctly passed
`TRANSPORT_LE`. The post-bond call fires the moment bonding completes — precisely when CTKD
has just created a dual-transport bond, the exact condition that made BlueZ pick BR/EDR — so
against a dual-mode peer (any desktop, and macOS especially) Android could connect over
classic, which serves no GATT. The direct analogue of the bug that cost the 2026-07-25
session; both paths now pass `TRANSPORT_LE`.

The Android↔Windows and Android↔Linux hotspot rows in the test plan are the ones that
exercise it.

### 2. ~~Android fails silently when the service or characteristics are missing~~ — fixed

Four exits from `onServicesDiscovered` — three printing nothing, none calling
`bluetoothFailed()`, no timeout behind them. Linux errors, Windows retries then errors, Apple
calls `cleanUpTransfer()`; Android hangs. Any of the bugs above, hit on Android, presents as
"it just hangs" with nothing in the log. Fixed in `6b29695`; all four exits now report and call
`bluetoothFailed()`.

**But that left the callback *before* it silent, which is worse.** A GATT connect that never
succeeds never reaches `onServicesDiscovered` at all, and `onConnectionStateChange`'s
disconnect branch was a bare `Log.i` that ignored `status` — so a failed connect showed up as
the UI going quiet immediately after "Stopped scanning", with the one number that names the
cause thrown away. Observed 2026-07-25 on Linux→Android.

It also never called `close()`. Android leaks the underlying client if you don't close it after
a disconnect, `connectGatt()` starts returning status 133 once enough have leaked, and
`Bluetooth.stop()` only closes `bluetoothGatt` — which a *failed* connect never assigns. So
each failed attempt leaked a client and each retry leaked another, which is the mechanism that
turns one transient failure into a device that stays broken until the app restarts. Both fixed;
the disconnect branch now closes, reports `status`, and distinguishes the three benign
disconnects (post-exchange, mid-bonding, and a stale overlapping connection) from a real one.

**Diagnostic value:** if Android goes quiet right after "Stopped scanning", it is failing to
connect, not failing to find the service — those are different callbacks with different causes.

That diagnostic immediately paid for itself, and against the previous entry: on the next run the
connect *succeeded* and the hang did not reproduce. The failure had moved one stage later, which
is the next item.

### 2a. ~~A joining Android never set `exchangeComplete`, so the peer's teardown read as a failure~~ — fixed 2026-07-25

Android→Linux succeeded, then Linux→Android reached `Joining flyingCarpet_79e9` — the exchange
was **complete**, the password was in hand — and two seconds later printed "Did not find the
Flying Carpet service on the peer" and aborted. The peer had done nothing wrong. Linux, having
handed over its credentials, sleeps one second, drops its GATT service and disconnects, exactly
as `core/src/linux/bluetooth.rs` intends. Android's central saw the Service Changed indication,
re-ran `discoverServices()`, got 3 services instead of 4, and took the missing-service exit that
`6b29695` had just wired to `bluetoothFailed()`.

The re-discovery in `onServiceChanged` (added in `b84911b`) is guarded on `exchangeComplete`
precisely to prevent this. The guard never fired, because **`exchangeComplete` was only ever set
on the hosting path** (`connectToPeer()`), and this device was joining. Android is central when
receiving and joins whenever the peer is Linux or Windows — so "Android receives from a PC", one
of the most ordinary configurations there is, ran every transfer with all three
`exchangeComplete` guards disarmed, including the post-exchange guard added in `3d2545a` one
commit earlier. Two of the three would have aborted this transfer; the service-changed one
simply got there first.

Fixed at the flag, in two places, rather than at either call site:

- `gotPassword()` sets it. That is the joiner's last BLE step, and both joiner roles pass
  through it — the central by reading the characteristic, the peripheral by having it written —
  so one assignment covers both. Not on an empty password: that means the peer's hotspot isn't
  up yet, and the replay this flag suppresses is the retry that recovers.
- `bluetoothFailed()` returns early when it is set. The teardown arrives as a Service Changed,
  then a disconnect, then whatever a read or write already in flight returns, and those reach
  three different call sites out of about ten. Once the transfer is on Wi-Fi, no BLE event should
  be able to kill it — gate the teardown once instead of auditing every caller.
- `Bluetooth.stop()` clears it, not just `scan()`. `scan()` runs for the central role only, so a
  peripheral transfer following a completed one would otherwise inherit `true` and ignore real
  failures for its whole duration. This is the bug the fix above would have introduced.

**The lesson is the flag's name.** `exchangeComplete` was set where the *host* finishes, and
read in three places that all meant "is BLE done with this transfer". Any predicate consumed by
guards that fail open needs to be true for every role that reaches them, and the roles here are
four independent axes (§1) — "hosting" is not "central" is not "sending".

### 2b. ~~Every GATT request Android issues could be dropped without a trace~~ — fixed 2026-07-25

Visible in the same log, and benign only by luck: the first `onServiceChanged` (Linux *adding*
its service) arrived while the `discoverServices()` from `onConnectionStateChange` was still in
flight, so two discoveries completed 13 ms apart and both called `read(OS)`. The second
`readCharacteristic()` returned false — queue busy — and `read()` ignored the return. One chain
died silently. It didn't matter because the two chains were identical, but two concurrent walks
of read-OS → write-OS → connectToPeer is not a state this code reasons about, and a silently
dropped GATT request is this project's signature failure mode.

`read()` had **three** ways to do nothing at all, none printing anything:

| | why it was silent |
|---|---|
| `bluetoothGatt` null | `bluetoothGatt?.readCharacteristic(...)` — the `?.` swallowed the whole call |
| characteristic null | unknown UUID fell through a `when` with no `else` |
| `readCharacteristic()` false | return value discarded |

`write()` had the same three, plus a `characteristic!!` on the Tiramisu path that would have
thrown rather than reported. And `discoverServices()` reports busy identically — false return,
no callback, nothing logged.

All of them now print. **Reported, not fatal**, deliberately: a false return is legitimately
transient here, because the two GATT connections after bonding are *meant* to coexist (§2) and
can each be walking the exchange, so one finding the queue busy is not grounds for killing a
transfer the other is about to finish. The overlap itself is gone too — both discovery call sites
go through one `startDiscovery()` that refuses to start a second while one is outstanding, and
clears the flag on connect, on disconnect, and in `onServicesDiscovered` before its permission
gate so it can never latch on.

**The general rule for this platform:** every Android GATT call is asynchronous *and* fallible
synchronously. The boolean or `BluetoothStatusCodes` return is the only notice you get that the
callback you are about to wait for will never arrive. Discarding it is how a transfer comes to
hang with an empty log — the same shape as §2, one layer down.

### 2c. ~~A completed transfer flipped the Bluetooth switch off~~ — fixed 2026-07-25

Observed running the release test plan's iOS → Android repeat-transfer row: both transfers
worked, and between them the Bluetooth switch turned itself off — the reliable
`bluetoothFailed()` indicator (§6), firing after a transfer that had finished cleanly. The
logcat trace made the mechanism unambiguous:

1. Fresh pairing mid-transfer means **two** GATT clients by design (§2): the pre-bond
   connect from the scan and the post-bond connect from the bond receiver. `bluetoothGatt`
   tracks only the most recently connected one.
2. `Bluetooth.stop()` closed only `bluetoothGatt` — one `close()`/`unregisterApp()` in the
   log, two clients in existence. The pre-bond client stayed registered and kept the ACL up
   (the GATT server logged "Device connected" again *immediately after* stop()). Law 9,
   Android edition.
3. iOS's own teardown (`removeService`) then delivered a Service Changed indication to the
   leftover client — *after* stop() had cleared `exchangeComplete`, so the §2a guard was
   disarmed. Re-discovery found 9 services instead of 10, took the missing-service exit,
   and `bluetoothFailed()` turned the switch off. The stranded client was still alive a
   full transfer later: during leg 2, *two* clients logged the peer's status-19 disconnect.

Fixed twice over, because the two halves cover different windows:

- **Every `connectGatt()` return is tracked** (`openConnections`) and `stop()` closes them
  all — a closed client delivers no callbacks, which is what makes stop() final.
- **`tearingDown`** is true from `stop()` until the next `scan()`/`advertise()` and gates
  `bluetoothFailed()` the same way `exchangeComplete` does, but for the between-transfers
  window that `exchangeComplete` cannot cover *because stop() clears it*. It also short-
  circuits `onServiceChanged` and the reconnect-rediscovery path, so a late peer teardown
  is a log line ("Ignoring service change after teardown"), not a discovery of a database
  with no Flying Carpet service in it.

**The lesson generalizes §2a's:** a guard cleared at teardown protects nothing that happens
after teardown. Every event the peer can still send after `stop()` — Service Changed,
disconnect, an in-flight read completing — needs a gate whose lifetime matches the gap
between transfers, not the transfer.

### 3. ~~The two unilateral-unpair paths violate law 4~~ — fixed 2026-07-25

Windows had **eight** `central.unpair()` sites, not one: enumeration failure (both branches),
plus every characteristic read and write. Six of those fired *after* enumeration had already
succeeded, where the bond is demonstrably fine and the link is up — a failing read is a timing
or peer-side problem that dropping the bond cannot fix, and the peer is left holding a key we
discarded. Those six are gone. The one that remains has positive evidence the bond is at
fault (a *reused* bond that failed every enumeration attempt) and now tells the user to remove
the pairing on the other device too.

Linux's poisoned-bond `remove_device` was written for the bearer problem that
`ensure_le_link()` now solves without touching the bond, so it was demoted from first rung to
last: retry once with the bond intact, and only then remove and re-pair, with the same
warning.

### 4. ~~Android and Apple never invalidate their GATT cache~~ — Android fixed 2026-07-25, Apple fixed in `4c59af6`

Android's `onServiceChanged` now calls `discoverServices()`, gated on `exchangeComplete` — the
TODO asked whether enabling it causes problems, and it does if ungated, because
`onServicesDiscovered` restarts the credential exchange. That's the same re-entrancy hazard
`onConnectionStateChange` already guards the same way.

Apple was once written up as fixed when it was not (verified stub at `328dfc8`); the real fix
is `FlyingCarpetApple` `4c59af6` (2026-07-25): a shared `didModifyServices` in
`Apple/shared/Bluetooth.swift` re-discovers when our service is among the invalidated ones, and both
targets' delegate methods now call it (macOS previously did not implement the method at all).
**Caveat: written on Windows, not yet compiled — verify with an Xcode build on both targets
before treating this as closed.**

**Remaining limitation, by design:** this only helps when the peer *sends* a Service Changed
indication. If it doesn't, neither stack learns its cache is stale. Android's only other lever
is the hidden `BluetoothGatt.refresh()` via reflection — non-public API, breaks across
versions, discouraged by Play policy — so the gap is documented rather than papered over.

### 5. Still open: nothing blocking

The `0x8000FFFF` enumeration failure against a bonded iPhone (`§3`, the original subject of
`docs/windows-ble-gatt-0x8000ffff.md`) remains unexplained and unreproduced since 2026-07-24.
The retry rung covers it; the unpair rung behind it is now the only unpair left in the
codebase.

---

## 6. Debugging playbook

| Symptom | Most likely cause | First thing to check |
|---|---|---|
| Connected, but **zero** services | link never encrypted — one-sided bond | is the peer still bonded? `bluetoothctl paired-devices` |
| Connected, services present, **ours missing** | stale GATT cache, or peer hadn't registered it yet | did the peer register the service *before* you connected? |
| Error containing **`br-`** | classic bearer chosen for an LE-only service | force LE; check bond provenance |
| Scan never finds an advertising peer | filtering on cached rather than advertised data | does the filter read advertisement data? |
| Works on first pairing, fails on reuse | bonded-vs-unbonded divergence | test both bond provenances (see below) |
| **"Already connected"**, then a ~120 s stall and a retry that repeats verbatim | a link inherited from the previous transfer, on the wrong bearer (§3a) | did either side `disconnect()` last time? is the guard checking `Connected` where it means `ServicesResolved`? |
| Android goes quiet right after **"Stopped scanning"** | the GATT *connect* failed — not service discovery, which reports every exit | `adb logcat -s Bluetooth` for the status in `onConnectionStateChange`; 133 after repeated attempts means leaked clients, so restart the app |
| A BLE error **seconds after** the credentials were exchanged — "services changed" then a missing service, or a disconnect | the peer's deliberate teardown, not a failure; a post-exchange guard that isn't set for this device's role (§2a) | is `exchangeComplete` set on *this* role's path? does the peer remove its service and disconnect after handing over credentials? |
| Bluetooth switch flips itself off and the peer-OS chooser reappears | `bluetoothFailed()` ran — that is `enableBluetoothUi(false)`, and nothing else in the app does it | a reliable "a BLE callback failed the transfer" indicator. Since §2c it can only fire *during* a transfer — a flip after a finished one is a §2c regression (a leftover client or a disarmed `tearingDown` gate) |
| macOS: **CBError 14** | peer deleted its half of the bond | law 4 |
| Windows: `0x8000FFFF` on enumeration | unresolved; retry ladder | `docs/windows-ble-gatt-0x8000ffff.md` |

**Always test both bond provenances.** From fully unpaired, run A→B then B→A; then unpair
both sides and run B→A then A→B. These are *different code paths* and only one of them was
passing for the entire v10 cycle.

**Useful instruments:** `sudo btmon` (raw HCI — the only way to see which bearer and what's
actually in the advertisement), `bluetoothctl info <addr>` (what BlueZ believes),
`adb logcat -s FlyingCarpet` (Android), `cargo tauri dev` stdout on both desktops. UI logs
alone were insufficient for every bug in this session.
