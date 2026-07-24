# Windows BLE: failures enumerating GATT services of an already-bonded peer

Findings from investigating the 2026-07-24 v10 test failure (Windows→iPhone hotspot
succeeded with fresh pairing; the reversed leg iPhone→Windows failed). Written to answer:
is the unpair-and-re-pair recovery in `core/src/windows/bluetooth.rs` a justified fix, or
a workaround papering over an unexamined bug?

> **Two distinct failures are described here — don't merge them.** The `0x8000FFFF`
> HRESULT exception against an iPhone (the original subject, mechanisms 1–3) is unresolved
> and intermittent. The Windows↔Linux failure of 2026-07-25 looks superficially similar in
> the UI but is a *different bug with a known cause* — Linux was deleting its half of the
> bond after every successful transfer with a non-macOS peer — and is fixed. See "Field
> observations".

**Short answer:** the failure matches a family of well-documented Windows BLE stack
problems with *bonded* peripherals, at least two independent mechanisms plausibly apply
to Flying Carpet's exact scenario, and unpairing is a precedented (community-endorsed)
recovery. But Microsoft's own guidance for the closest-matching mechanism is a plain
**retry loop, no unpairing** — so a cheaper retry rung before the unpair rung might avoid
the re-pair PIN dialog in some or all recurrences. That retry rung is now implemented
ahead of the unpair rung, with UI-visible diagnostics that will show which rung fixes
real-world occurrences (see "Recovery ladder" below).

## Observed failure

Sequence:

1. **Transfer 1** (Windows sends → iPhone receives): Windows is BLE peripheral, iPhone is
   central. Fresh 6-digit pairing during the transfer. Transfer completes. The iOS app
   then runs `stopBluetooth()` (`iOS/FlyingCarpet/ViewController.swift`): disconnects the
   BLE link and — important — `removeService()` deletes the Flying Carpet GATT service
   from the iPhone's GATT database.
2. **Transfer 2** (iPhone sends → Windows receives): roles reverse. The iOS app re-adds
   the service (`addServiceIfNeeded()` in `shared/Bluetooth.swift`) and advertises;
   Windows scans as central, recognizes the bonded device ("Already BLE paired with
   Bluetooth device"), skips pairing, and calls
   `GetGattServicesWithCacheModeAsync(BluetoothCacheMode::Uncached)`
   (`core/src/windows/central.rs`, `get_services_and_characteristics`). That call throws
   HRESULT `0x8000FFFF` (`E_UNEXPECTED`, "Catastrophic failure") — as an exception, not a
   `GattCommunicationStatus` failure — on its first and only attempt.
3. The (pre-fix) error path unpaired and aborted. A manual rerun then paired fresh
   (6-digit PIN) and worked.

Conditions that make this scenario unusual, all worth keeping in mind as contributing
factors:

- The bond was created **role-reversed**: Windows was the GAP *peripheral* when the
  pairing happened, so Windows had never performed GATT discovery against the iPhone
  under this bond before transfer 2.
- The iPhone's GATT database **changed between connections** (FC service removed after
  transfer 1, re-added before transfer 2) — the exact case the GATT *Service Changed*
  mechanism exists for on bonded devices.
- iPhones always use **LE privacy** (resolvable private addresses that rotate); the bond
  gives Windows the IRK to resolve them.

## Field observations

**2026-07-24, second run (passed, ladder in place, `cargo tauri dev` with console):**
same sequence re-run: fresh 6-digit pairing during the Windows→iPhone leg, then
iPhone→Windows reusing the bond. Enumeration succeeded on **attempt 1** — no retry, no
unpair, no PIN dialog. Notable details from stdout:

- The reused bond had the **same role-reversed provenance** as the one that failed
  (created while Windows was the peripheral), and it enumerated fine — direct evidence
  against mechanism 3 ("role-reversed bonds are inherently defective").
- `AlreadyPaired` came from the **connected short-circuit**: no "weren't connected" /
  pairing prints appeared, so the leg-1 BLE link was still up when leg 2 scanned. Windows
  never had to reconnect, so no RPA-resolution window ever opened. First confirmation
  that this branch occurs in practice — and that GATT client enumeration over the
  leftover reverse-role link works.
- Refined hypothesis: the failure window is when Windows must **reconnect** to the
  bonded LE-privacy peripheral (link dropped between transfers — e.g. a longer idle
  gap); when the previous link survives into the next transfer, enumeration works. This
  predicts failures correlate with time elapsed between the two legs, and squares with
  bleak #1771 (a reconnect-to-paired-device scenario).
- Side confirmation: `watcher wasn't started. status:
  BluetoothLEAdvertisementWatcherStatus(0)` shows `stop_watching()` right after `scan()`
  is a no-op (status still `Created`), so the Received handler stays registered — the
  assumption `rescan()` relies on holds on real hardware.

**2026-07-25, Windows ↔ Linux (failed — and it is a different bug):** Windows→Linux
succeeded with fresh pairing; the reversed leg Linux→Windows failed. The ladder fired for
the first time, and what it found says the ladder was reasoning about the wrong thing.

The distinguishing evidence is a line that is *absent* from the log.
`get_services_and_characteristics` prints `UUID: {…}` once per service as it iterates
(`core/src/windows/central.rs`). Between `got services` and `Flying Carpet service not
found in peer's service list`, the log has **none** — so the service collection was
**empty**. That is not "connected, but the peer's database lacks our service"; it is "no
GATT database was read at all". Attempt timings 1.4 s / 7.7 s / 0.5 s, with the 7.7 s
attempt showing the connect-timeout shape.

**Root cause, and it is not in the Windows stack:** `core/src/linux/bluetooth.rs` used to
end a successful transfer by removing the peer's bond unless the peer was macOS
(`keep_bond = info.0 == "mac"`). In leg 1 Linux was the central, so on completion it did
`adapter.remove_device()` and **dropped its half of the bond**. Windows kept its half, so
leg 2 hit `AlreadyPaired`, skipped pairing, and tried to encrypt the link with an LTK the
Linux box had forgotten. The link never encrypted, so there was no GATT database to read,
and WinRT reported that as success-with-an-empty-list rather than a failure status.

The premise behind that removal — recorded in the code as "Windows/Android re-pair per
transfer, so removing is safe" — is false for **both** platforms it names. Windows'
central path short-circuits on `AlreadyPaired` and reuses the bond; the Android code
contains no `removeBond` call anywhere. macOS was never a special case. It was the first
platform where the symptom happened to be legible, because CoreBluetooth names it
(CBError 14, "Peer removed pairing information") instead of silently returning nothing.

Consequences for this document's conclusions:

- The iPhone→Windows failure at the top of this doc is `0x8000FFFF`, an HRESULT
  *exception*. This one is a clean call returning an empty list. **Different failures**;
  mechanisms 1–3 below were derived from the former and do not explain the latter.
- The ladder's rung 1 (retry) cannot help a one-sided bond — no number of retries
  re-creates a key the peer discarded. Rung 2 (unpair + re-pair) is the *right shape* of
  fix, but it fired only after the retries and then failed to re-pair, leaving both sides
  worse off: `Pairing result: Failed` on that attempt and on every subsequent transfer.
- Reporting "Flying Carpet service not found in peer's service list" for an empty list is
  what pointed the ladder at the peer's GATT database instead of at the link. Fixed:
  `get_services_and_characteristics` now checks `GattDeviceServicesResult::Status()` before
  reading `Services()`, and reports an empty-but-successful list as its own condition,
  naming a one-sided bond as the likely cause.

**Still unexplained:** why re-pairing then failed repeatedly (`Pairing result: Failed`
across three transfer attempts), and why the ThinkPad remained listed under Windows'
Bluetooth devices after `UnpairAsync` returned `Unpaired`. `UnpairAsync` clears the
association belonging to the `DeviceInformation` it was called on; Windows can hold a
separate association record for the same physical device, which would keep it listed and
could keep a stale pairing state in play. `unpair()` now logs `IsPaired` afterward so the
next occurrence says whether the bond this path cares about actually went away. Settling
it needs Linux-side evidence too — `bluetoothctl paired-devices` / `btmon` during a failed
re-pair — which no log in this document has yet.

**Manual recovery, until a run confirms the fix:** remove the pairing on *both* sides —
Windows Settings → Bluetooth → Remove device, and `bluetoothctl remove <address>` on Linux
— then pair fresh. Removing only one side reproduces the original condition.

The `0x8000FFFF` recovery ladder has still **not been exercised against the failure it was
built for**; the iPhone case remains intermittent and unreproduced since.

## External evidence

- **[Microsoft Q&A: Retry required for GetGattServicesAsync() when connecting to BLE
  peripheral using LE privacy](https://learn.microsoft.com/en-us/answers/questions/2280559/retry-required-for-getgattservicesasync()-when-con)**
  — the closest match. With a *paired* peripheral using LE privacy, the first
  `GetGattServicesAsync()` reliably fails: Windows' first connection attempt goes to the
  peripheral's **unresolved random address** and fails; the RPA/IRK resolution
  (`BthIRKResolution` in Event Viewer) completes internally and the *next* attempt
  succeeds. Microsoft's response confirms no public API exposes the resolution event and
  **officially recommends a retry loop** (~3 attempts, ~1 s delay, Uncached). Unpairing
  is not mentioned — not needed in that thread.
- **[Nordic DevZone: WinRT code works if device NOT paired, fails Unreachable if
  paired](https://devzone.nordicsemi.com/f/nordic-q-a/48916/bluetooth-le-windows-10-using-winrt-c-code-works-if-device-not-paired-fails-with-unreachable-if-device-is-paired)**
  — same shape from the other direction: everything works *until* the device is bonded.
  Never root-caused; the thread's own workaround is "pair, record it, then **unpair** to
  get the thing connecting" — i.e., the same family of workaround this repo uses.
- **[bleak #1771: Fail to connect to paired device](https://github.com/hbldh/bleak/issues/1771)**
  — reconnecting to a paired, private-advertising device on Windows leaves the GATT
  session in `CLOSED`; "works when unpairing before disconnecting." Independent
  confirmation that *bonded + LE privacy + reconnect* is the problem combination in the
  WinRT stack, and that unpairing sidesteps it.
- **[bleak #1061: access denied due to GattServicesChanged](https://github.com/hbldh/bleak/issues/1061)**
  — when the remote's GATT database changes, Windows fires `GattServicesChanged` and
  **clears all cached services**; API calls racing that window fail with access-denied /
  "method called at an unexpected time" errors. Relevant because the iPhone's database
  genuinely changed between bond creation and transfer 2, so a Service Changed
  indication lands right around Windows' reconnection and discovery.
- **[Microsoft Q&A: E_UNEXPECTED accessing the same device from two applications](https://learn.microsoft.com/en-us/answers/questions/584108/c-bluetooth-windows-10-accessing-the-same-device-f)**
  and **[Windows-universal-samples #1209](https://github.com/microsoft/Windows-universal-samples/issues/1209)**
  — establish that this exact HRESULT (`0x8000FFFF`) does come out of
  `GetGattServicesAsync`/connect paths under stack-internal contention, distinct from the
  cleaner `Unreachable` status.
- Background on why database changes on bonded devices are hairy:
  [Silicon Labs on Service Change Indication](https://docs.silabs.com/bluetooth/latest/bluetooth-gatt/service-change-indication)
  — bonded clients cache attribute handles and rely on Service Changed indications on
  reconnect; stacks differ in how gracefully they handle the invalidation window.
- Retry/timing as the general community remedy for first-attempt discovery failures on
  Windows: bleak [#60](https://github.com/hbldh/bleak/issues/60),
  [#740](https://github.com/hbldh/bleak/issues/740),
  [#825](https://github.com/hbldh/bleak/issues/825),
  [#1217](https://github.com/hbldh/bleak/issues/1217),
  [#1340](https://github.com/hbldh/bleak/issues/1340).

## Candidate mechanisms, ranked

1. **RPA/IRK resolution race** (best-supported): Windows' first GATT connection attempt
   to a bonded LE-privacy peripheral targets the unresolved random address and fails;
   resolution completes asynchronously; a second attempt succeeds. Predicts that a plain
   retry — same device object, same bond — would succeed. Unpairing also "fixes" it, but
   only incidentally: an unbonded connect just uses the advertised RPA directly.
2. **GATT database-changed flux**: the iPhone removed and re-added the FC service between
   the two connections under the bond. On reconnect, Windows processes the Service
   Changed indication by clearing its service state mid-discovery; calls racing that
   window fail. Also fixed by a delay+retry; also incidentally fixed by unpairing (a
   fresh bond has no stale database state to invalidate).
3. **Bond made in the reverse role is itself defective** (the original codebase theory:
   "windows has trouble enumerating services of already-paired devices"): possible, and
   unpair+re-pair is the only cure if true, but no external report specifically matches
   the role-reversal detail. The Nordic and bleak threads implicate *bonded + privacy*
   generally, not bond provenance.

Honest caveats: this is one observed occurrence, with UI logs only (no stdout, no
timing, no packet capture). The observed failure is an HRESULT *exception*, while the
best-matched Microsoft thread describes a `GattCommunicationStatus::Unreachable`
*status* on a 7-second timeout — same scenario, different failure surface, so the match
is strong but not exact. Mechanisms 1 and 2 are inferred from external reports, not
proven against this hardware.

## Assessment of the shipped recovery

The recovery added on 2026-07-24 (in `negotiate_bluetooth`, central branch): on
enumeration failure against an already-paired device, unpair → rescan → re-pair (new PIN
confirmation) → retry discovery, once; bond still persists across successful transfers.

- **Soundness: good.** It automates exactly the manual sequence that verifiably worked,
  it's the same workaround two independent community threads landed on, and it fires only
  on failure — the happy path (healthy bond reuse, no dialog) is unchanged and identical
  to other platforms.
- **Minimality: unproven.** If mechanism 1 or 2 is the real cause, a retry of
  `GetGattServicesWithCacheModeAsync` (Microsoft's own recommendation) would recover
  *without* destroying the bond or showing a PIN dialog. The recovery ladder below was
  added to settle this empirically.

## Recovery ladder (implemented 2026-07-24)

`negotiate_bluetooth` (central branch, `core/src/windows/bluetooth.rs`) now escalates
through:

1. **Retry rung** — up to 3 enumeration attempts, ~1 s apart, per the Microsoft Q&A
   guidance. "Service list returned but Flying Carpet service missing" is treated as
   retryable too (mechanism 2 predicts transiently incomplete lists), so
   `get_services_and_characteristics` no longer unpairs internally on that result — the
   ladder owns the unpair decision.
2. **Unpair rung** — if all attempts fail and the bond was reused: unpair, rescan, pair
   fresh (new PIN confirmation), then run the retry rung once more under the new bond.
   If that also fails, unpair and abort as before.

What the UI log records on a failure, and how to read it:

- `Couldn't read Bluetooth services (attempt N/3, took X.Xs): <error>` — one line per
  failed attempt. The timing distinguishes an immediate stack rejection (<1 s) from the
  ~7 s connect-timeout shape of mechanism 1; the error text distinguishes HRESULT
  failures from a missing-service list.
- `Diagnostic info: {reused|new} bond; peer {was already|was not} connected when
  discovered` — printed with the first failure; settles which `AlreadyPaired` branch
  fired (the still-connected short-circuit vs. reconnect-to-bonded-device).
- `Reading Bluetooth services succeeded on attempt N` — the money line. If this appears,
  a benign mechanism (1/2) is confirmed for that occurrence and no PIN dialog was
  needed.
- `Couldn't read services of already-paired Bluetooth device. Unpairing and pairing
  again...` — the unpair rung fired; if the transfer then completes, that occurrence is
  evidence the bond itself was the problem (mechanism 3).

Interpretation over time: if field logs show rung 1 succeeding, the unpair rung can be
demoted to rarely-hit insurance; if rung 1 never succeeds and rung 2 always does,
mechanism 3 (the role-reversed bond is itself defective) becomes the leading theory and
re-pairing is genuinely required. stdout still carries extra detail if a console is
attached (e.g. `RequestAccessAsync`'s result in the scan callback).
