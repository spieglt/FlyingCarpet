# Audit: hardcoded delays across both repos

Date: 2026-07-25. Scope: every `sleep`, `delay`, `Task.sleep`, `Thread.sleep`,
`asyncAfter`, `postDelayed`, and bounded `timeout` in `FlyingCarpet` (Rust, Kotlin, JS) and
`Apple/` (Swift). 45 sites found, plus 4 commented-out ones.

## Verdict

**The situation is better than it looks, and the problem is concentrated, not spread out.**

Of 45 delay sites, **31 are legitimate** — retry backoff, bounded timeouts on genuinely
asynchronous OS state, and protocol-defined announce intervals. **2 are test-only.**
**12 are fixed settling delays**: "wait N seconds and hope the other side is ready."

Those 12 are not scattered. **Nine of them are in the BLE credential exchange**, in all
three languages, and they exist because that exchange has one structural gap: *it is a
sequence of GATT reads and writes with no completion signal*, so every platform independently
bolted a timer onto it. The other three are the tail of the file-transfer confirmation
handshake, which has the same shape of gap.

So this is not "hacky fixes all over the place." It is **two protocol gaps, papered over
independently six times.** That's a much more tractable thing to fix — and also a reason not
to fix it by deleting sleeps one at a time.

Nothing here is a v10 blocker. Recommendations are split accordingly at the end.

---

## The 12 fixed settling delays

Ranked by how confident I am that they're papering over something, and what they cost.

### 1. The BLE credential exchange — 9 sites, ~4–6 seconds per transfer

| File | Delay | Stated reason |
|---|---|---|
| `core/src/linux/central.rs` `find_characteristics` | 2 s | none — bare `sleep` before bonding |
| `core/src/linux/central.rs` `exchange_info` ×4 | 1 s each | none — one after each characteristic read/write |
| `core/src/linux/bluetooth.rs` `negotiate_bluetooth` | 1 s | "Removing GATT service" — hold it open a moment longer |
| `core/src/windows/bluetooth.rs` `negotiate_bluetooth` | 1 s | "keep everything in scope until peer has had a chance to read the password" |
| `Android/…/Bluetooth.kt` `onCharacteristicRead` | 1 s | peer's SSID came back empty; sleep and re-read |
| `Android/…/Bluetooth.kt` `onConnectionStateChange` | 1.6 s | before `discoverServices()` |

This is the delay observed in testing on 2026-07-25: a Linux central spends **4 s (joining)
to 6 s (hosting)** in pure sleeps before the hotspot is even created, with nothing on screen.

Three distinct things are going on:

**(a) "Empty SSID means not ready yet" is a polling protocol.** The peripheral publishes an
empty string until its hotspot exists; the central reads, sees `""`, waits, and reads again.
Android sleeps 1 s and re-reads. iOS and macOS schedule a re-read (2 s / 1 s). All three are
working around the same thing: the SSID characteristic never notifies. A GATT
notify/indicate on that characteristic would delete this entire category, on every platform
at once — the central would be told, rather than asked to guess. That is the single
highest-value change in this document.

**(b) Post-operation settling on Linux.** The four 1 s sleeps in `exchange_info` and the 2 s
before bonding have **no comment and no stated cause**. The adjacent comment explains
something else (that `WriteOp::Request` is used because iOS ignored unconfirmed writes),
which suggests these were added while chasing the same iOS problem and then never revisited
after the real fix landed. They are the most likely to be pure superstition — and also the
riskiest to remove blind, because if any of them is load-bearing it will only show up
against Apple peers.

**(c) Hold-open-before-teardown.** Windows and Linux both sleep 1 s before dropping the GATT
service so the peer can finish reading. But both already wait for an event that says the
read happened (`PeerReadPassword` / `BluetoothMessage::Password`) immediately before
sleeping. Worth checking whether the event already guarantees what the sleep is buying; if
so these are free to delete.

**Android's 1.6 s before `discoverServices()` deserves special mention** — its own comment
says the delay was *not* the fix:

> `// this was the reason android couldn't connect to macOS? no, was the setLegacy(false).`

That is a sleep whose author recorded, in the code, that it didn't solve the problem it was
added for. It survived anyway.

### 2. Android sleeps on the Bluetooth callback thread — a real bug, not just a delay

`Android/…/Bluetooth.kt`, in `onCharacteristicRead`:

```kotlin
outputText("Could not read peer's WiFi characteristic. trying again...")
Thread.sleep(1000)
read(SSID_CHARACTERISTIC_UUID)
```

This blocks the BLE callback thread for a full second. Every other GATT callback queued
behind it — including ones for an unrelated in-flight operation — is stalled.

The Apple side hit this exact problem and documented the fix, in both `ViewController.swift`
files:

> `// scheduled rather than slept: this callback runs on the bluetooth queue, and sleeping`
> `// here stalls every other callback behind it`

…and uses `queue.asyncAfter` instead. **Android should do the same** — post a delayed
runnable rather than sleeping. Same bug class, one platform learned it and the knowledge
didn't cross over. `Thread.sleep(1600)` before `discoverServices()` is in the same callback
context and has the same problem.

This is the one item in the audit I'd call an outright defect rather than a smell.

### 3. The end-of-transfer confirmation — 3 sites, and the code says so

`core/src/receiving.rs`, twice, verbatim:

```rust
// TODO: ugly hack to get around lifetime issue? sending end didn't receive this last
// reply when calculating hash of large file.
sleep(time::Duration::from_secs(1)).await;
```

The author's own note, question mark included. The surrounding protocol is a "double
confirmation" at the end of each file: the sender writes a u64, the receiver replies, the
sender replies again. It has no clean termination condition, so all three platforms bolt a
timer on:

- Rust receiver, end of `receive_file`: `timeout(2 s)` on the final read, then prints
  "Didn't receive confirmation" and carries on
- Swift receiver, end of `receiveFile`: a detached task that sleeps 2 s and calls `killIt()`
- Rust receiver, in `check_for_file`: the two 1 s sleeps above, after replying "I don't have
  this file" — held open because the sender missed that reply while it was busy hashing a
  large file

None of these is dangerous — the file data is already written and verified by then, and the
timeout path is handled — but three independent timers around one handshake is the signature
of a protocol that doesn't say when it's finished. Cheapest honest fix is to define an
explicit end-of-transfer message rather than inferring completion from silence. That is a
**wire-format change**, so it is v11 work, not v10.

### 4. iOS `joinHotspot()` — unbounded retry loop

`iOS/FlyingCarpet/ViewController.swift`:

```swift
while true {
    try await NEHotspotConfigurationManager.shared.apply(config)
    try? await Task.sleep(nanoseconds: 3_000_000_000)
    if Task.isCancelled { throw TransferError.UserCancelled }
    if await isConnected() { break }
    NEHotspotConfigurationManager.shared.removeConfiguration(forSSID: self.transfer.ssid)
}
```

The 3 s sleep is fine — association genuinely continues after `apply()` returns. The **loop
has no attempt limit**: it exits only on success or user cancellation. Compare
`Transfer.swift`'s peer-IP wait, which bounds itself at 120 iterations and throws
`CouldNotJoinNetwork`. Cancellation does provide an escape, so this is a consistency and
user-experience issue rather than a hang, but it should count attempts like its neighbour
does.

---

## The 31 legitimate ones

Recorded so a future audit doesn't re-litigate them.

**Retry backoff after a failed operation** (7): TCP connect retries in `core/src/lib.rs`,
`MainViewModel.kt`, and `Transfer.swift` (2 s each — three platforms, consistent); `nmcli
con up` retry in `core/src/linux/network.rs` (1 s); hotspot join retry in
`core/src/windows/network.rs` (2 s) and `Transfer.swift` (2 s); GATT enumeration retry in
`core/src/windows/bluetooth.rs` (1 s, matching Microsoft's published guidance —
see `docs/windows-ble-gatt-0x8000ffff.md`).

**Polling OS state that fires no event** (6): gateway discovery after joining a hotspot
(200 ms on both Windows and Linux); characteristic-discovery retry on Linux (2 s);
transfer-cancellation wait in the Tauri command (100 ms); discovery-cancellation poll in
`core/src/discovery.rs` (100 ms); peer-IP wait in `Transfer.swift` (1 s, **bounded at 120**).

**Protocol-defined announce intervals** (6): `DISCOVERY_INTERVAL_MS` in `discovery.rs`,
`Discovery.kt`, and `Discovery.swift`. These are the wire protocol, identical across
platforms by design — not delays in the sense this audit is about.

**Bounded timeouts on external events** (8): LE bonding socket, 60 s (waits on a human
confirming a PIN); final-confirmation read, 2 s; discovery socket receive, 100 ms; firewall
rule verification, 10 × 500 ms (netsh runs in a separate elevated process, so the result is
genuinely asynchronous — and it's a bounded poll with a warning on failure, which is the
right shape); iOS local-network permission probe, 3 s; `Network.swift` connection-waiting
cancel, 30 s (with a comment explaining precisely why the bound exists); `Network.swift`
generic connect timeout; Swift discovery poll, 50 ms.

**Correctly scheduled rather than slept** (2): the iOS and macOS SSID re-reads via
`queue.asyncAfter`. These are the pattern Android should copy.

---

## Test hygiene finding (unrelated to delays, found on the way)

`core/src/linux/network.rs` has two tests that manipulate **real system network state** and
neither is `#[ignore]`d:

- `start_and_stop_hotspot` — creates a real NetworkManager hotspot named
  `flyingCarpet_1234`, sleeps 5 s, tears it down
- `join_hotspot` — calls the real join path, sleeps 20 s, tears down

So `cargo test` on a Linux machine tries to reconfigure the network, needs polkit auth, and
can leave a stale `flyingCarpet_*` connection behind — which is exactly the residue that
issue **#51** is about.

Their Windows counterparts (`join_hotspot`, `check_for_firewall_rule`) *are* both
`#[ignore]`d; the second was marked in this very branch (`dbdfbb1`) for the identical
reason. The Linux pair was missed. **This one is a two-line fix and worth doing before
release**, since it can actively dirty a test machine.

---

## Recommendations

### Before v10

1. **Mark the two Linux hardware tests `#[ignore]`**, matching the Windows pattern. Two
   lines, no behavior change, stops `cargo test` from dirtying a Linux box.
2. **Fix Android's `Thread.sleep` on the BLE callback thread** — replace both with a posted
   delayed runnable, copying the Apple approach. This is a defect with a known-correct fix
   already implemented on another platform, and it's in the credential-exchange path that
   has been the source of most release-test failures.
3. **Bound the iOS `joinHotspot()` loop** with an attempt limit, like its neighbour.

Everything else should wait. Sleeps in the BLE path are precisely the code that current
hardware testing is validating, and removing them mid-test-cycle invalidates results for no
user-visible gain.

### After v10

4. **Add notify/indicate to the SSID characteristic.** This is the root fix: it removes the
   "read, see empty, wait, re-read" pattern from Android, iOS, and macOS simultaneously, and
   is the reason several of the settling delays exist at all. Wire-compatible in the sense
   that it adds a capability rather than changing existing bytes — but it touches all three
   implementations and needs the usual cross-platform care.
5. **Audit the five unexplained Linux BLE sleeps individually**, on hardware, against an
   Apple peer specifically — that's the peer they were most likely added for. Remove them
   one at a time, not as a batch, so a regression identifies itself.
6. **Check whether the two hold-open-before-teardown sleeps are already covered** by the
   read events that immediately precede them.
7. **Give the transfer an explicit end-of-transfer message** instead of inferring completion
   from silence, retiring the three confirmation timers and the two "ugly hack" sleeps.
   Wire-format change — **v11**, per the version rule in `CLAUDE.md`.

### Not recommended

Do not do a sweeping "remove all the sleeps" pass. Several of these are load-bearing in ways
that only show up against one specific peer platform, the failure mode is an intermittent
hang rather than a clean error, and the code has no automated coverage at this layer — every
regression would be found by hand, on hardware, weeks later.
