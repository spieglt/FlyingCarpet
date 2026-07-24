# Audit: bonded vs. unbonded BLE behavior on every platform

Date: 2026-07-25. Prompted by three Windows↔Linux failures in a row, all of which turned out
to be the same shape: **code that works against an unbonded peer and behaves differently, or
not at all, against a bonded one.** This audit asks where else that shape exists.

Read `docs/windows-ble-gatt-0x8000ffff.md` first for the three failures that motivated this.

## Why this class of bug is systemic here

Two facts combine badly:

1. **Every platform's BLE peripheral removes its GATT service when a transfer ends.** iOS
   calls `removeService()`, Linux drops the GATT `app_handle`, Windows releases the
   `GattServiceProvider`, Android closes and reopens its GATT server. The "Lifecycle status"
   matrix in the release test plan tracks this as a *feature* — and for advertising and
   connection teardown it is.
2. **Every platform's BLE central caches the peer's GATT database, keyed to the bond.** That
   is what a bond is partly *for*: skip rediscovery next time.

So the peer's GATT database legitimately changes between transfers, and the central is
legitimately holding a cached copy of the old one. The GATT spec's answer is the *Service
Changed* indication. **Two of our three central implementations receive that notification and
throw it away.**

Until now this was masked: Linux deleted its bond after every transfer, so there was rarely a
cached database to go stale. Fixing that (`6039d53`) makes the bonded path the normal path on
the pairing that gets tested most.

---

## Findings

### 1. Android never invalidates its GATT cache — highest risk

`Android/…/Bluetooth.kt`:

```kotlin
override fun onServiceChanged(gatt: BluetoothGatt) {
    super.onServiceChanged(gatt)
    …
    outputText("Services changed")
    // TODO: should this be enabled? does it cause problems? https://developer.android.com/…
    // gatt.discoverServices()
}
```

The callback that exists precisely to handle "the peer's database changed" logs a line and
does nothing. Its one meaningful action is commented out behind an unanswered question.

Android caches the GATT database for **bonded** devices specifically — `discoverServices()`
returns the cache rather than re-reading from the peer — and there is no public API to clear
it (the usual workaround is the hidden `BluetoothGatt.refresh()` via reflection, which
appears nowhere in this codebase).

**Predicted symptom:** Android as central, second or later transfer with a bonded peer whose
service was removed and re-added — `getService(SERVICE_UUID)` returns null, or returns a
service whose characteristic handles are stale and whose reads fail.

**Status: predicted from code, not observed.** Android↔Windows and Android↔Linux hotspot
rows are still unchecked in the test plan, and they are exactly the ones that would show
this. Worth running deliberately: transfer, then transfer again without restarting either
app, with Android receiving both times.

### 2. Android fails silently when the service or characteristics are missing

Same function, and this is what makes finding 1 dangerous rather than merely annoying:

```kotlin
val service = gatt.getService(SERVICE_UUID)
if (service == null) {
    outputText("Did not find service")
    return                                                    // no bluetoothFailed(), no retry
}
osCharacteristic       = service.getCharacteristic(OS_CHARACTERISTIC_UUID)       ?: return
ssidCharacteristic     = service.getCharacteristic(SSID_CHARACTERISTIC_UUID)     ?: return
passwordCharacteristic = service.getCharacteristic(PASSWORD_CHARACTERISTIC_UUID) ?: return
```

Four exits that leave the transfer waiting forever for a credential exchange that will never
happen. Three of them print nothing at all. There is no timeout behind them.

Compare the other three centrals, all of which fail loudly:

| Platform | Missing service → |
|---|---|
| Linux | `ServicesUnresolved` error after a bounded retry loop |
| Windows | retry ladder, then a named error naming the likely cause |
| iOS / macOS | `cleanUpTransfer()` with a user-facing message |
| **Android** | **silent return; hangs** |

This is the same silent-hang class as the Linux `scan()` bug fixed in `ed921ac`, and it is
the most likely way finding 1 would present to a user: "it just hangs."

**This is the highest-value fix in this document** — it is small, it is independent of
whether finding 1 is real, and it converts an unbounded hang into a diagnosable error.

### 3. Apple discards the same notification

`iOS/FlyingCarpet/ViewController.swift`:

```swift
func peripheral(_ peripheral: CBPeripheral, didModifyServices invalidatedServices: [CBService]) {
    print("invalidatedServices: \(invalidatedServices)")
}
```

Implemented, prints, does not re-discover. **macOS does not implement it at all** — so the
two Apple targets differ from each other, which is its own small inconsistency worth closing
regardless.

CoreBluetooth caches services per `CBPeripheral` and persists them for bonded peers, so the
same staleness applies. `Bluetooth.swift`'s `scan()` also short-circuits through
`retrieveConnectedPeripherals(withServices:)`, which returns a peripheral object that may
already carry a populated (and stale) `services` array.

**Severity is lower than Android's** for one reason: both Apple targets now handle read
failures in `didUpdateValueFor`, reporting the error and calling `cleanUpTransfer()`. A stale
handle surfaces as a failed read with a message, not a hang. That error handling is doing
load-bearing work it wasn't written for.

Minor, same file: `didDiscoverPeripheral` force-unwraps `discoveredPeripheral!` — a
different variable than the `peripheral` parameter it was handed. It is set by the
ViewControllers immediately before, so it works, but it is a crash waiting for a reordering.

### 4. Windows — checked, and clean on the things I suspected

Recorded so this isn't re-audited later:

- **`peer_device` latch resets correctly.** The scan callback bails with "we've already
  initiated pairing" if `peer_device` is already set, which would be a stale-state hazard
  across transfers — but `BluetoothCentral::new()` runs per transfer and `rescan()` clears it.
  Not a bug.
- **Cache mode is explicit and correct.** `GetGattServicesWithCacheModeAsync(Uncached)` — the
  one central that asks for a fresh read. Since this session it also checks
  `GattDeviceServicesResult::Status()` before trusting the list.
- Windows is therefore the only central of the four that both bypasses the cache *and*
  validates the result.

What remains on Windows is the still-unexplained `0x8000FFFF` against a bonded iPhone, and
the re-pair that fails permanently afterward — both already documented, neither reproduced
since.

### 5. Bond provenance changes cached state — generalize the Linux lesson

The Linux hang in `ed921ac` came down to *how* the bond was created: bonding while acting as
central left a device record containing our service UUID; bonding while acting as peripheral
left one without it, and the scan dismissed it forever.

The general statement — worth holding while testing any platform — is that **a bond formed
while you were the peripheral leaves you with a different cached view of the peer than one
formed while you were the central**, because the record was built from an incoming connection
rather than from an advertisement you filtered for. Flying Carpet reverses these roles between
transfers by design (sender = peripheral, receiver = central), so both provenances occur
routinely, and each platform's discovery code was written and tested against the central-first
one.

That is why the test plan now runs Windows↔Linux from unpaired in **both** starting orders.
The same doubling is worth doing for Android↔Windows and Android↔Linux.

---

## Recommendation

The individual fixes are worth doing, but there is a root-cause fix worth considering first.

### Root cause: stop removing the GATT service between transfers

Every finding above exists because the peripheral's GATT database changes between transfers.
If the service were registered once for the app's lifetime and only **advertising** were
started and stopped, no central would ever hold a stale database, and the invalidation gaps
on Android and Apple would stop mattering.

Windows already separates these — `012d00b` added an explicit `stop_advertising()` while the
service provider lives until drop. The other platforms tear down both together.

**The catch, and it is a real one:** the SSID and password characteristics would remain
readable between transfers. They are encryption-gated (bonded peers only), but a bonded peer
could read a previous transfer's credentials. Any move in this direction must clear those
characteristic values at teardown rather than leaving the last transfer's in place. That is
a small change, but it is a security-relevant one and needs stating explicitly in review —
the hotspot password is the Noise PSK.

I'd treat this as post-v10: it touches the BLE lifecycle on four platforms, and the release
test plan's entire lifecycle tier was written against the current teardown behavior.

### For v10

1. **Make Android's four silent returns loud** (finding 2). Call `bluetoothFailed()` with a
   message naming the missing service or characteristic. Small, self-contained, and it
   converts the most likely field failure from an unbounded hang into a bug report that says
   what happened. Do this one regardless of everything else.
2. **Run the two Android hotspot pairs twice each, without restarting** (findings 1 and 5) —
   Android↔Windows and Android↔Linux, Android receiving both legs. These rows are already
   unchecked in the test plan; this audit just says what to watch for.
3. **Implement `didModifyServices` on macOS**, matching iOS, even if both only log for now.
   Costs nothing and removes a gratuitous difference between two targets that share
   `Bluetooth.swift`.

### After v10

4. Decide the root-cause question above: keep services registered and clear their values, or
   implement invalidation on Android (`onServiceChanged` → `discoverServices()`) and Apple
   (`didModifyServices` → re-discover). Keeping the service registered is less code in more
   places; the invalidation route is more conservative and testable per platform.
5. Fix the `discoveredPeripheral!` force-unwrap in `didDiscoverPeripheral`.
