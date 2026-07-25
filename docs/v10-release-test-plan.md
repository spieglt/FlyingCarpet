# Flying Carpet v10 — Release Test Plan

Branch: `shared-network` (both repos: `FlyingCarpet`, `FlyingCarpetApple`).
Goal: validate everything v10 introduces before release, efficiently, by gating on
builds/KATs first and exploiting the symmetry of the wire protocol.

## What v10 introduces (scope to validate)

- **Shared Network mode** — transfer over an existing WiFi/wired network instead of a
  hotspot. HMAC-authenticated discovery, manual password (receiver generates + shows QR,
  sender types/scans), no Bluetooth, TCP receiver = server on port 3290.
- **Noise encryption over both modes** — `Noise_NNpsk0_25519_ChaChaPoly_SHA256`, PBKDF2
  PSK, plaintext version/mode preamble bound into the Noise prologue. Replaces the old
  inner AES-GCM. Now encrypts *all* metadata (filenames, sizes, count), not just contents.
- **Protocol bumped to v10 (breaking)** — a v10 device talking to v9 must show a clear
  version-mismatch message, not hang or misbehave.
- **Discovery key derived from the PBKDF2-stretched PSK** (no fast hash of the password
  on the air). Generated passwords bumped 8 → 10 chars.
- **Interface chooser** — dropdown labeled with IP, hides unusable interfaces, supports
  wired interfaces (shared network).
- **macOS ↔ Linux Bluetooth fix** — Linux LE-bonds before connecting; pairing agent
  confirms the 6-digit code in-UI. macOS RPA / "CBError 14" bond-keeping.
- **Bug-fix bundle** — UI-freeze drop guard, stale hotspot cleanup (#51), Android
  permissions-vs-hardware (#101), Windows profile XML injection fix, receive-path hardening.
- **Late lifecycle fixes (this branch tail)** — Android hotspot flag + GATT client/server
  close; Apple BLE connection/service teardown; coroutine-cancellation no longer reported
  as an error. See "Lifecycle status" below.
- **Send Folder is consistent across all five platforms** — a sent folder is now recreated
  inside the receiver's chosen destination everywhere, instead of only on macOS. Fixes an
  Android→desktop failure and two desktop abort cases along the way. Retest rows in Tier 6;
  details in `docs/send-folder-behavior.md`.

## Platforms & environment

Five platforms: **Windows** (10+), **Linux** (AppImage + .deb), **macOS**, **iOS**,
**Android** (API 29+). Apple devices cannot host a hotspot, so **Apple↔Apple must use
shared network**. Ideally have two Android and/or two desktops available for same-OS runs.

---

## Tier 0 — Build & static gates (do first)

Cheapest, and catches the biggest current risk: **the Apple lifecycle changes have never
been compiled.**

- [x] Android: `./gradlew assembleDebug` (and `lintDebug` — only the pre-existing
      `MainActivity:94` MissingPermission finding is expected)
- [x] Rust core: `cargo build` for Windows and Linux; `cargo clippy` clean — "clean" means
      no errors. `cargo clippy --workspace --all-targets` on Windows emits 23 style lints in
      `core` and 4 in the app (`is_none()` over `== None`, `Ok(…)?`, `expect` with a format
      arg, arg counts); all pre-existing, none in v10 code paths.
- [x] Tauri desktop app builds on Windows and Linux (`cargo tauri build`)
- [x] **iOS builds in Xcode** (FlyingCarpetApple)
- [x] **macOS builds in Xcode** (FlyingCarpetApple)
- [x] Android unit tests pass: `NoiseUnitTest`, `DiscoveryUnitTest`
- [x] Rust unit tests pass: `cargo test` (incl. `official_noise_test_vector`,
      `wrong_password_fails_handshake`, `tampering_is_detected`, `round_trip_small_and_large`,
      and the `utils::selection_tests` folder-naming cases)
- [x] Apple unit tests pass (Noise KATs, incl. cacophony vector + app KATs)
- [x] Both repos report the **same protocol version constant** (wire-compat check).
      Re-verified 2026-07-25: Rust `MAJOR_VERSION: u64 = 10` (`core/src/lib.rs:83`), Kotlin
      `MAJOR_VERSION: Long = 10` (`MainViewModel.kt:56`), Swift `VERSION: UInt8 = 10`
      (`shared/Transfer.swift:17`). The Swift type is narrower but goes out as
      `Data([0,0,0,0,0,0,0,VERSION])`, so all three write the same 8 big-endian bytes.

---

## Tier 1 — Smoke (prove the pipeline, 2 transfers)

- [x] Shared network: iOS → Android (small file)
- [x] Hotspot: iOS → Android (small file)

---

## Tier 2 — Interop matrix

The wire protocol (discovery → preamble → Noise → files) is symmetric, so a directed
cycle covers each platform as **both** sender and receiver without testing every pair.
W=Windows, L=Linux, M=macOS, I=iOS, A=Android.

### Shared network — core cycle (each platform sends once, receives once)
- [x] W → L
- [x] L → M
- [x] M → I
- [x] I → A
- [x] A → W
- [x] I → M  (Apple ↔ Apple, the case that *requires* shared network)
- [x] M → M  or  A → A  (same-OS sanity, if a second device is available)

### Hotspot — each BLE stack pairs with each other, each non-Apple platform hosts
Apple always guests; the peer hosts. Confirm the 6-digit pairing code and the transfer.
- [x] A → I  and  I → A   (Android hosts; Android ↔ Apple BLE)
- [x] A → M  and  M → A   (Android hosts; Android ↔ macOS BLE)
- [x] W → I  and  I → W   (Windows hosts; Windows ↔ Apple BLE)
- [x] L → M  and  M → L   (Linux hosts; **macOS ↔ Linux — the v10 BLE fix**)
- [x] W → L  and  L → W   (desktop ↔ desktop hosting)
- [x] W → A  and  A → W   (Windows ↔ Android BLE)
- [x] L → A  and  A → L   (Linux ↔ Android BLE)

> If any single hop fails, expand *only that pair* to localize it. Passing the cycle +
> the hotspot pairs means every platform's discovery, Noise, BLE, and hotspot code has
> run in both roles.

---

## Tier 3 — Cryptography & protocol validation

- [x] KATs green on all three implementations (Rust, Android, Apple) — the real
      cross-platform interop guarantee. 2026-07-25: Rust (28 pass, 2 hardware-ignored) and
      Android (`NoiseUnitTest` 10, `DiscoveryUnitTest` 4) re-run; Apple's suite was run on
      a Mac in Tier 0 and its sources are unchanged since. All nine shared vectors — PSK,
      discovery key, app handshake msg1/msg2/record, prologue + its msg1/msg2/record — are
      byte-identical in `core/src/noise.rs`, `NoiseUnitTest.kt`, and
      `macOS/FlyingCarpetTests/FlyingCarpetTests.swift`, as is the discovery announcement
      vector between Rust and Kotlin.
      - Two asymmetries in *coverage*, not in the vectors: Swift has the discovery **key**
        KAT but no discovery **announcement** vector (the 108-byte layout + HMAC is pinned
        only between Rust and Kotlin), and the iOS test target holds only the Xcode
        template tests — the Noise KATs live in the macOS target. Both projects compile the
        same `shared/Noise.swift`, so iOS's implementation is covered as long as the macOS
        suite is run; nothing guards a Swift-side change to the announcement format.
- [x] **Wrong password** (shared network): enter a mismatching password → clear
      "could not establish a secure connection / check the password" message, no hang
- [ ] **Wrong password** (hotspot): same, via the BLE-exchanged password path
- [x] **Version mismatch**: run a v10 build against a v9 build → clear version-mismatch
      message on both ends, no hang or garbage
- [x] **Preamble tamper** (if a test hook exists / via unit test) — handshake fails,
      transfer aborts. Unit-tested on all three: `prologue_mismatch_fails_handshake` (Rust),
      `prologueMismatchFailsHandshake` (Kotlin), `testPrologueMismatchFails` (Swift). Each
      flips one bit of the responder's transcript and asserts the handshake fails even
      though the passwords match. `tampered_handshake_is_detected` covers a corrupted
      handshake message on all three as well.
- [x] Metadata confidentiality sanity: confirm filenames/sizes are no longer sent in the
      clear (packet capture optional; primarily covered by Noise KATs). Verified by
      inspection 2026-07-25: everything after the preamble goes through the
      `TransferStream::Encrypted` handle — file count (`lib.rs`), then filename length,
      filename bytes, size, per-chunk lengths and hashes (`sending.rs:87-117`). The only
      plaintext writes are the version/mode preamble, which the prologue binds, and the
      `TransferStream::Plain` fallback used solely to report a version/mode mismatch.

---

## Tier 4 — Lifecycle regression (the fixes on this branch tail)

Each row has a specific repro that previously failed — test the repro, not just "works."

- [x] **Android repeat transfer**: iOS → Android **twice in a row**, no restart →
      transfer 2 stands up its hotspot; no "hotspot already running" in logcat; the
      Bluetooth switch stays **on** between legs (2026-07-25: a leftover pre-bond GATT
      client received iOS's teardown Service Changed after stop() and flipped it off —
      field guide §2c; expect "Ignoring service change after teardown" in logcat instead)
- [x] **Android GATT teardown**: back-to-back hotspot transfers → no phantom
      "Wrote OS to peer" / "Device connected" churn between transfers (logcat)
- [x] **Apple central teardown**: macOS/iOS as **receiver** (central), then a second
      transfer → no leaked connection; second transfer clean
- [x] **Apple peripheral service**: Apple as **sender** twice in a row → service
      re-registers; second advertise/read works
- [ ] **Cancel mid-transfer** (Android, hotspot) → UI re-enables; no spurious
      "Transfer error: … cancelled"
- [ ] **Cancel mid-transfer** (Android, shared network) → no spurious "Transfer error"
      or "Discovery error: … cancelled"
- [x] **Successful shared-network receive** (Android) → **no** "Discovery error" after
      "Transfer complete"
- [ ] **Cancel mid-transfer** (each desktop) → UI recovers (drop-guard); hotspot torn down
- [ ] **Stale BLE pairing recovery**: forget the peer on one side only, retry → re-pairs
      cleanly (do not need to forget both). **Known to fail as of 2026-07-25** — this is the
      unresolved half of the Windows↔Linux failure; the re-pair returns
      `Pairing result: Failed` and keeps failing on subsequent attempts. Capture
      `bluetoothctl paired-devices` and `btmon` on the Linux side when testing this.
- [x] **Bidirectional BLE hotspot, Windows ↔ Linux** (the 2026-07-25 repro): Windows → Linux,
      then immediately Linux → Windows without restarting either app. Leg 2 must reuse the
      bond and enumerate services. Previously leg 1 made Linux delete its half of the bond,
      so leg 2 got an empty GATT service list and then failed to re-pair permanently. Confirm
      `bluetoothctl paired-devices` on Linux still lists the Windows box after leg 1.
- [x] **Both bond provenances, Windows ↔ Linux.** Run the pair twice from fully unpaired,
      once in each starting order, because the two produce *different* cached state on Linux
      and only one of them used to work:
      - [x] unpaired → **Windows → Linux** first (Linux bonds as central) → then Linux → Windows
      - [x] unpaired → **Linux → Windows** first (Linux bonds as peripheral) → then
            Windows → Linux. This is the order that hung: Linux's cache entry for the peer had
            no Flying Carpet UUID, and the scan only reacted to `DeviceAdded`, which never
            fires twice for a bonded peer. Expect `Found peer … by re-reading known devices`
            in the Linux stdout.
      - [ ] Then a third leg in each case, to confirm repeat transfers keep working
- [ ] **Bidirectional BLE hotspot, Linux ↔ Android** and **Linux ↔ macOS**, same pattern —
      Linux no longer removes any peer's bond, so all three pairings need one clean
      round trip. macOS was already exempt and should be unchanged.
- [x] **Android post-bond bearer** (`TRANSPORT_AUTO` → `TRANSPORT_LE`, fixed 2026-07-25):
      Android ↔ **macOS** and Android ↔ Linux/Windows, from fully unpaired, so the post-bond
      `connectGatt` runs against a dual-mode peer right after cross-transport key derivation.
      This was the direct analogue of the Windows↔Linux `br-connection-canceled` bug and had
      never been exercised. Failure looks like a connect that succeeds with no Flying Carpet
      service.
- [ ] **Android as central, twice in a row, bonded** — Android↔Windows and Android↔Linux,
      Android **receiving** both legs, no app restart between them. This row was written when
      Android never invalidated its GATT cache and predicted a silent hang; both halves of
      that prediction have since been fixed (`onServiceChanged` now re-discovers, gated on
      `exchangeComplete`, and every `onServicesDiscovered` exit reports and calls
      `bluetoothFailed()`). Expected now: leg 2 logs "Services changed" followed by a
      successful re-discovery and a normal transfer. A "Did not find the Flying Carpet
      service" abort or any hang after "Discovered services" is a regression in that fix,
      not the previously predicted stale-cache hang. See `docs/ble-bond-asymmetries.md`.
- [ ] **Poisoned-bond self-heal still works** (Linux as central): the deliberate
      `remove_device` on characteristic-discovery failure was kept; confirm a genuinely bad
      bond still recovers via the "retrying with a fresh pairing" path.
- [ ] **Android rotation mid-transfer**: rotate during a *multi-file* transfer (enough files
      that the log has scrolled) → transfer keeps running; the log survives whole, with no
      duplicated or missing line at the seam and no truncation; auto-scroll still follows new
      lines afterward; progress bar and button states are preserved. The log now lives in the
      ViewModel instead of the saved-state `Bundle`, so this also covers the
      `TransactionTooLargeException` that a long transfer's log previously risked on rotation.
      Rotate a second time after the transfer completes to confirm the finished log persists.

---

## Tier 5 — Platform-specific

- [x] **Android**: LocalOnlyHotspot works on target device (known-broken on some
      Xiaomi/MIUI/HarmonyOS — note device model tested)
- [ ] **Android**: deny then re-grant Bluetooth permission → switch stays usable, recovers
      on resume (#101); password-prompt dialog readable in dark mode
- [ ] **iOS**: after a hotspot transfer, no leftover `flyingCarpet_*` Wi-Fi config; force-
      quit mid-transfer then relaunch → stale config removed on startup
- [ ] **Linux**: force-kill mid-hotspot, relaunch → stale `flyingCarpet_*` NetworkManager
      connection removed on startup (#51)
- [ ] **Windows**: WiFi Direct AP starts/stops cleanly; SSID with special characters does
      not break profile handling (XML-injection fix); firewall prompt handled
- [ ] **macOS**: long transfer doesn't silently drop when macOS switches back to an
      internet network (known caveat — confirm behavior/messaging)
- [x] **Interface chooser** (desktop): dropdown lists interfaces with IPs, hides unusable
      ones, wired interface works in shared network mode
- [ ] **Shared network over a manual iPhone Personal Hotspot** joined by both devices
      (documented Apple↔Apple path)

---

## Tier 6 — Robustness / edge cases

- [x] Multi-file transfer (2+ files)
- [ ] Large single file (> 2 GB if feasible; sustained multi-record Noise streaming)
- [ ] Empty / zero-byte file
- [ ] Filename with Unicode / spaces / emoji
- [ ] Peer never starts → no infinite hang; cancellable; clean message
- [ ] Receiver started long before sender → still connects (no premature timeout)
- [ ] Sender and receiver both pick the same mode (both Send / both Receive) → clean
      "both sides picked the same mode" error
- [ ] Two transfers in a row in **shared network** mode on every platform (mirror the
      Android repeat-transfer regression)

### Send Folder — retest on all five (behavior changed; previously inconsistent)

Every platform now sends a chosen folder so the **receiver recreates that folder inside the
destination they picked, with the contents inside it**. Before this change only macOS did
that; Windows, Linux, Android, and iOS dumped the contents loose into the destination, and
Android→desktop failed outright for any folder with sub-folders. Rationale, the old
per-platform behavior, and the fixes: `docs/send-folder-behavior.md`.

Use one test folder throughout, so results are comparable. It must exercise every case that
used to break:

```
TestFolder/
  top.txt                  <- file directly inside the selection
  Nested/inner.txt         <- one level down
  Nested/Deeper/deep.txt   <- two levels down
  OnlyDirs/x/1.txt         <- selection level with NO loose files (used to flatten or fail)
  OnlyDirs/y/2.txt         <- sibling of the above (used to abort: "Strip prefix error")
```

Pass = destination contains `TestFolder/` with all five files at the paths above, and
**nothing** loose in the destination root.

- [x] Windows sends `TestFolder` (Send Folder checkbox) → receiver gets `TestFolder/…`
- [x] Windows sends `TestFolder` by **drag-and-drop** onto the window → same result
- [x] Linux sends `TestFolder` (checkbox and drag-and-drop)
- [x] Android sends `TestFolder` (Send Folder checkbox) → **to a Windows or Linux receiver**;
      this is the combination that used to fail with "Received invalid filename path"
- [ ] iOS sends `TestFolder` ("Send Folder" in the "Send from:" prompt)
- [ ] macOS sends `TestFolder` (choose the folder in the picker) → confirm **no regression**;
      this is the one platform whose behavior did not change
- [x] Each of the five **receives** `TestFolder` from at least one other platform, and the
      folder is recreated rather than flattened
- [ ] Plain multi-file selection still arrives **flat** (no folder created) on all five —
      the fix must not wrap ordinary file sends in a directory
- [ ] Desktop: select/drop files from **two different directories** at once → all arrive,
      flat, no "Strip prefix error" (previously aborted the transfer)
- [ ] Desktop: drop **two folders** at once → both recreated side by side (previously aborted)
- [ ] Send a folder twice into the same destination → second copy lands under "(1) name"
      siblings rather than clobbering
- [ ] Folder containing a file with Unicode / spaces / emoji in a **sub-folder** name

---

## Lifecycle status (BLE + hotspot resource teardown)

Post-fix state. ✅ = correct; ⚠️ = fragile/by-design; see caveats.

| Platform | Hotspot torn down | Central: scan stopped | Central: connection closed | Peripheral: advertising stopped | Peripheral: service removed |
|---|---|---|---|---|---|
| **Android** | ✅ fixed | ✅ | ✅ **fixed** | ✅ | ✅ **fixed** |
| **Windows** | ✅ | ✅ | ⚠️ persists (by design) | ✅ **fixed** (explicit StopAdvertising) | ✅ (registration released on drop) |
| **Linux** | ✅ | ✅ (RAII) | ✅ (RAII + remove_device) | ✅ (explicit drop) | ✅ (explicit drop) |
| **iOS** | ✅ | ✅ | ✅ **fixed** | ✅ | ✅ **fixed** |
| **macOS** | ✅ | ✅ | ✅ **fixed** | ✅ | ✅ **fixed** |

**Verification status of the fixes:**
- Android fixes: compile + lint verified; **not yet hardware-tested** (Tier 4 rows).
- Apple fixes: **not yet built** (Tier 0) and not tested (Tier 4 rows).
- Windows peripheral advertising now stops explicitly (`StopAdvertising()` +
  `RemoveAdvertisementStatusChanged`); the one remaining ⚠️ is the device
  connection/pairing persisting after a transfer, which is **by design** (Windows has
  trouble re-enumerating already-paired devices, so "unpair after every transfer" is
  intentionally disabled) and is **pre-existing**, not a v10 regression.
  **Hardware-tested 2026-07-24:** Windows→iPhone hotspot (fresh pairing) passed, but the
  reversed second leg (iPhone→Windows, reused bond) failed GATT service enumeration with
  `0x8000FFFF`; a manual rerun with fresh pairing succeeded. The Windows central now
  recovers from this automatically: on enumeration failure it retries enumeration (up to
  3×, ~1 s apart), and only if the bond was reused and all retries fail does it unpair,
  rescan, and re-pair (new PIN confirmation) within the same transfer instead of
  aborting. **Retest 2026-07-24: passed** — reversed second leg reused the bond and
  enumerated services on attempt 1 (leg-1 BLE link was still up, so no reconnect was
  needed); no recovery rung fired, so the ladder itself remains field-unexercised. The
  original failure is intermittent — if it recurs, the UI log's attempt/timing
  diagnostics will show which rung fixed it. Root-cause investigation, sources, and the
  recovery-ladder design: `docs/windows-ble-gatt-0x8000ffff.md`.
- Linux is the reference implementation (full RAII); no action.

**Release read on the lifecycle front:** the code gaps found in the audit are closed on
Android and Apple, so on paper every platform is ✅ except Windows' pre-existing ⚠️. But
"buttoned up" requires the verification: **Tier 0 build of Apple + Tier 4 regressions on
Android and Apple must pass.** Until then the fixes are unverified. Windows' remaining
items are not new in v10 and should not block release.

---

## Release gate

- [x] Tier 0 fully green (**hard blocker**: Apple must build)
- [x] Tier 1 green
- [x] Tier 2 core cycle + hotspot pairs green
- [ ] Tier 3 green (wrong password + version mismatch are must-pass)
- [ ] Tier 4 green (lifecycle regressions — highest-risk new code)
- [ ] Tier 5 green or documented known-issue per platform
- [ ] Tier 6 green or documented
- [ ] Version strings bumped to 10 in all artifacts; changelog/README updated. Versions
      checked 2026-07-25 and all agree: `core/Cargo.toml`, `src-tauri/Cargo.toml`, and
      `tauri.conf.json` at 10.0.0; Android `versionName "10.0.0"` (`versionCode 22`); iOS and
      macOS app targets `MARKETING_VERSION = 10.0.0` (the 1.0 entries in both pbxprojs belong
      to the test bundles, which don't ship). README covers Shared Network + Noise. Left
      unchecked for the changelog only — the repo has no `CHANGELOG.md`, so if release notes
      live on the GitHub Releases page, that's the remaining item.
- [ ] Both repos tagged in lockstep (wire-compatible)
