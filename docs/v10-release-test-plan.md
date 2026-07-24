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
- [x] Rust core: `cargo build` for Windows and Linux; `cargo clippy` clean
- [x] Tauri desktop app builds on Windows and Linux (`cargo tauri build`)
- [x] **iOS builds in Xcode** (FlyingCarpetApple)
- [x] **macOS builds in Xcode** (FlyingCarpetApple)
- [x] Android unit tests pass: `NoiseUnitTest`, `DiscoveryUnitTest`
- [x] Rust unit tests pass: `cargo test` (incl. `official_noise_test_vector`,
      `wrong_password_fails_handshake`, `tampering_is_detected`, `round_trip_small_and_large`)
- [x] Apple unit tests pass (Noise KATs, incl. cacophony vector + app KATs)
- [x] Both repos report the **same protocol version constant** (wire-compat check)

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
- [ ] M → M  or  A → A  (same-OS sanity, if a second device is available)

### Hotspot — each BLE stack pairs with each other, each non-Apple platform hosts
Apple always guests; the peer hosts. Confirm the 6-digit pairing code and the transfer.
- [ ] A → I  and  I → A   (Android hosts; Android ↔ Apple BLE)
- [ ] A → M  and  M → A   (Android hosts; Android ↔ macOS BLE)
- [ ] W → I  and  I → W   (Windows hosts; Windows ↔ Apple BLE)
- [ ] L → M  and  M → L   (Linux hosts; **macOS ↔ Linux — the v10 BLE fix**)
- [ ] W → L  and  L → W   (desktop ↔ desktop hosting)
- [ ] W → A  and  A → W   (Windows ↔ Android BLE)
- [ ] L → A  and  A → L   (Linux ↔ Android BLE)

> If any single hop fails, expand *only that pair* to localize it. Passing the cycle +
> the hotspot pairs means every platform's discovery, Noise, BLE, and hotspot code has
> run in both roles.

---

## Tier 3 — Cryptography & protocol validation

- [ ] KATs green on all three implementations (Rust, Android, Apple) — the real
      cross-platform interop guarantee
- [ ] **Wrong password** (shared network): enter a mismatching password → clear
      "could not establish a secure connection / check the password" message, no hang
- [ ] **Wrong password** (hotspot): same, via the BLE-exchanged password path
- [ ] **Version mismatch**: run a v10 build against a v9 build → clear version-mismatch
      message on both ends, no hang or garbage
- [ ] **Preamble tamper** (if a test hook exists / via unit test) — handshake fails,
      transfer aborts
- [ ] Metadata confidentiality sanity: confirm filenames/sizes are no longer sent in the
      clear (packet capture optional; primarily covered by Noise KATs)

---

## Tier 4 — Lifecycle regression (the fixes on this branch tail)

Each row has a specific repro that previously failed — test the repro, not just "works."

- [ ] **Android repeat transfer**: iOS → Android **twice in a row**, no restart →
      transfer 2 stands up its hotspot; no "hotspot already running" in logcat
- [ ] **Android GATT teardown**: back-to-back hotspot transfers → no phantom
      "Wrote OS to peer" / "Device connected" churn between transfers (logcat)
- [ ] **Apple central teardown**: macOS/iOS as **receiver** (central), then a second
      transfer → no leaked connection; second transfer clean
- [ ] **Apple peripheral service**: Apple as **sender** twice in a row → service
      re-registers; second advertise/read works
- [ ] **Cancel mid-transfer** (Android, hotspot) → UI re-enables; no spurious
      "Transfer error: … cancelled"
- [ ] **Cancel mid-transfer** (Android, shared network) → no spurious "Transfer error"
      or "Discovery error: … cancelled"
- [ ] **Successful shared-network receive** (Android) → **no** "Discovery error" after
      "Transfer complete"
- [ ] **Cancel mid-transfer** (each desktop) → UI recovers (drop-guard); hotspot torn down
- [ ] **Stale BLE pairing recovery**: forget the peer on one side only, retry → re-pairs
      cleanly (do not need to forget both)
- [ ] **Android rotation mid-transfer**: rotate during a *multi-file* transfer (enough files
      that the log has scrolled) → transfer keeps running; the log survives whole, with no
      duplicated or missing line at the seam and no truncation; auto-scroll still follows new
      lines afterward; progress bar and button states are preserved. The log now lives in the
      ViewModel instead of the saved-state `Bundle`, so this also covers the
      `TransactionTooLargeException` that a long transfer's log previously risked on rotation.
      Rotate a second time after the transfer completes to confirm the finished log persists.

---

## Tier 5 — Platform-specific

- [ ] **Android**: LocalOnlyHotspot works on target device (known-broken on some
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
- [ ] **Interface chooser** (desktop): dropdown lists interfaces with IPs, hides unusable
      ones, wired interface works in shared network mode
- [ ] **Shared network over a manual iPhone Personal Hotspot** joined by both devices
      (documented Apple↔Apple path)

---

## Tier 6 — Robustness / edge cases

- [x] Multi-file transfer (2+ files)
- [x] Whole-folder transfer (nested; common-folder placement matches other platforms)
- [ ] Large single file (> 2 GB if feasible; sustained multi-record Noise streaming)
- [ ] Empty / zero-byte file
- [ ] Filename with Unicode / spaces / emoji
- [ ] Peer never starts → no infinite hang; cancellable; clean message
- [ ] Receiver started long before sender → still connects (no premature timeout)
- [ ] Sender and receiver both pick the same mode (both Send / both Receive) → clean
      "both sides picked the same mode" error
- [ ] Two transfers in a row in **shared network** mode on every platform (mirror the
      Android repeat-transfer regression)

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
  intentionally disabled) and is **pre-existing**, not a v10 regression. Compile-verified
  on the Windows target; not yet hardware-tested.
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
- [ ] Tier 2 core cycle + hotspot pairs green
- [ ] Tier 3 green (wrong password + version mismatch are must-pass)
- [ ] Tier 4 green (lifecycle regressions — highest-risk new code)
- [ ] Tier 5 green or documented known-issue per platform
- [ ] Tier 6 green or documented
- [ ] Version strings bumped to 10 in all artifacts; changelog/README updated
- [ ] Both repos tagged in lockstep (wire-compatible)
