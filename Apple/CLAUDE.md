# Flying Carpet — Apple (iOS + macOS)

The **Swift** iOS and macOS apps. Port **3290** throughout, same wire protocol as the Rust core
and the Android app. See the root `CLAUDE.md` for the repo-wide picture and
`../ARCHITECTURE.md` for the connection role model.

**Build only on a Mac.** Nothing here compiles as part of the Cargo workspace, and nothing in
`core/` targets Apple platforms — the Swift code is a full independent port of the protocol, not
a binding over the Rust core. The **Rust core is the reference** this port is tested against.

## Layout

- `shared/` — protocol code shared by both apps (added to both Xcode targets):
  - `Transfer.swift` — the orchestrator. `runTransfer()` entry point, connection-mode logic,
    password generation, `Transfer.Delegate` protocol (the per-platform ViewController
    implements it).
  - `Discovery.swift` — shared network peer discovery: multicast + unicast UDP on port 3290
    over raw POSIX sockets, then TCP on 3290.
  - `Network.swift` — TCP read/write in a `TCPConnectionProtocol` extension; `TCPClient`
    (hotspot) and `TCPConnectionWrapper` (shared). `AtomicBool` is defined here once.
    Connections run on `networkQueue`, not main.
  - `Noise.swift` — `Noise_NNpsk0_25519_ChaChaPoly_SHA256` handshake + `buildPrologue`.
  - `Send.swift` / `Receive.swift` — sending/receiving halves.
  - `Bluetooth.swift` — BLE password exchange (**hotspot mode only**, see invariants).
- `iOS/FlyingCarpet/` — iOS app. `ViewController.swift` (main UI + BLE + picker delegates),
  `QRScanner.swift` / `QRScanner.storyboard` (camera QR scanning), `Base.lproj/Main.storyboard`.
- `macOS/FlyingCarpet/` — macOS app. `ViewController.swift`, `SsidBox.swift`,
  `Base.lproj/Main.storyboard`.

## Build

```
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  xcodebuild -project <iOS|macOS>/FlyingCarpet.xcodeproj -scheme "FlyingCarpet" build
```

For a specific destination: iOS `-destination 'generic/platform=iOS Simulator'`, macOS
`-destination 'platform=macOS'`.

**Code signing:** `DEVELOPMENT_TEAM` is deliberately blank in both `project.pbxproj` files, so a
fresh clone fails with *"Signing for 'FlyingCarpet' requires a development team."* Either select
your team once in Xcode (target → Signing & Capabilities → Team; Xcode writes it back into
`project.pbxproj` as a local modification) or pass it per-build without touching the file:

```
xcodebuild ... DEVELOPMENT_TEAM=YOURTEAMID -allowProvisioningUpdates build
```

**The tests live in the macOS target.** `macOS/FlyingCarpetTests/FlyingCarpetTests.swift` holds
all 23 of them, including the cross-platform Noise and discovery known-answer vectors.
`iOS/FlyingCarpetTests/` is a 36-line stub — don't go looking for the KATs there, and run the
macOS test target when changing protocol code.

**SourceKit lies about single files.** Diagnostics that lint one file in isolation report bogus
"Cannot find type 'Transfer'/'Bluetooth' in scope" and "No such module 'UIKit'" errors — the
cross-file and cross-target types resolve fine in a real `xcodebuild`. Trust the build, not the
isolated-file lint.

## Architecture

- **Two connection modes:** `hotspot` (one device hosts an ad-hoc Wi-Fi network) and
  `sharedNetwork` (both already on the same LAN; discovery finds the peer over IP). Set from
  `connectionModeSwitch` (Hotspot=0 / Shared Network=1) on both platforms.
- **Receiver is the anchor.** In both modes the receiver generates the password, is the TCP
  server (Noise responder), and (shared network) displays the password; the sender is the TCP
  client (Noise initiator) and supplies the password.
- **Apple-to-Apple only works in shared network mode.** Apple no longer lets hotspots be
  configured programmatically, so hotspot mode is blocked for two Apple devices (error
  `appleToAppleHotspotErrorMessage`). Use a normal Wi-Fi network or a manually-configured
  Personal Hotspot the other device has joined.

## Load-bearing invariants (don't "simplify" these)

These mirror the root `CLAUDE.md`; the Swift-specific details are here.

- **v10 = Noise.** Every transfer (both modes) runs the Noise NNpsk0 handshake; the PSK is
  `PBKDF2-HMAC-SHA256(password, salt="Flying Carpet v10 shared network PSK", 600_000)`. Noise is
  the sole cipher. v9 peers are rejected.
- **Preamble → prologue binding.** Version/mode are negotiated in a plaintext preamble, then
  every preamble byte is bound into the Noise prologue. Both platforms of any pair must build
  the prologue identically. Cross-platform KATs guard this — keep the Swift Noise tests
  (`macOS/FlyingCarpetTests/FlyingCarpetTests.swift`) in sync with `../core/src/noise.rs` and the
  Kotlin `NoiseUnitTest`; discovery vector in sync with `../core/src/discovery.rs`'s
  `test_cross_platform_vector`.
- **Passwords: single-use + CSPRNG.** The receiver mints a fresh random password per transfer
  (`generatePassword()` — 10 chars from a 57-symbol confusables-free charset, matching the
  desktop/Android generators) and displays it; never reuse, never user-chosen, never remember.
- **Bluetooth is hotspot-only.** Shared network exchanges the password manually (display +
  type/QR); do **not** re-add BLE to shared mode. The BT switch is greyed out in shared network
  mode on both platforms — every site that re-enables it goes through
  `bluetoothSwitchShouldBeEnabled()` (`capable && connectionMode == .hotspot`), because
  `toggleUI` and `toggleBluetoothUI` both used to restore it on hardware capability alone.
  Apple-to-Apple can't pair iPhone↔Mac over BLE by design — exactly the pair that would need it.

## Password exchange & QR codes (shared network mode)

The receiver generates and displays the password; the sender supplies it. To avoid typing, the
receiver can display it as a **QR code** and the sender can **scan** it.

- **QR content format (must match across platforms):** shared network QR = the **bare
  password**. Hotspot QR = `"ssid;password"`. Scanners split on `;`: >1 component ⇒ hotspot
  (ssid + password), otherwise the bare password.
- **Receivers display the QR** two ways (like Android/desktop): in the main-screen logo image
  view (`logoImageView`, reset to the app logo in `toggleUI(transferRunning: false)`), and in a
  popup. macOS uses CoreImage `CIQRCodeGenerator` (`qrCodeImage(from:size:)` → `NSImage`) in the
  password `NSAlert`'s accessory view; iOS uses the same generator (→ `UIImage`) in a dedicated
  `QRDisplayViewController` (a `UIAlertController` can't host an image). Both also show the text
  password for a sender that can't scan.
- **iOS sender** offers "Scan QR Code" in the password prompt (`promptForPassword`), routing to
  `ScannerViewController` via the `goToQRScanner` segue; the scanned string flows through
  `codeScanned(result:)`, which sets the password and starts the transfer. iOS can also still
  type the password.
- **macOS has no scanner** (Macs aren't portable and may lack a camera). So a macOS *sender*
  types the password shown on the peer; iOS/Android senders can scan. Net: iOS↔macOS uses QR
  when iOS is the sender; macOS-as-sender types.

## Peer OS selection on Apple (why macOS has a `peerSwitch` and iOS doesn't)

On host-capable platforms the "Select Peer OS" control feeds `is_hosting(peer, mode)` — who
creates the hotspot (see `../ARCHITECTURE.md`, "Peer OS Selection"). **Apple devices never host
a hotspot** (no public API — the peer always hosts), so on Apple the peer's OS is *never* needed
to decide hosting. That collapses the control's job to two leftovers, both **hotspot-mode-
without-Bluetooth only**:

1. **Is the peer Android?** An Android hotspot's SSID is OS-assigned and not derivable from the
   password, so the joiner must be told the SSID separately (`promptForSsidAndPassword`); every
   other peer derives the SSID from the password (`getSsidAndKey`), so only the password is
   needed.
2. **Fast-fail Apple↔Apple.** Selecting a macOS/iOS peer in hotspot mode is impossible (neither
   can host), so macOS rejects it up front with the "use Shared Network mode" message instead of
   failing deep in the join.

Both platforms hide/skip peer OS exactly when it can't matter: it's irrelevant in **shared
network mode** (discovery + receiver-anchor fixes the roles) and is learned automatically over
the BLE **OS characteristic** when **Bluetooth** is on. macOS therefore hides `peerSwitch` in
shared network mode and when BT is active. **iOS has no peer switch at all**: it never hosts,
and it gets the Android SSID (and password) out-of-band by scanning the host's `ssid;password`
QR (`codeScanned`) or over Bluetooth — so it never needs the user to name the peer OS. macOS
keeps the switch only because it has no camera scanner and so needs the manual "peer is Android
⇒ ask for SSID" path plus the early Apple↔Apple rejection.

Implication if simplifying: macOS's 5-segment `peerSwitch` is really a binary "is the peer
Android?" plus an Apple↔Apple guard. It could be reduced to that (or dropped if macOS gained QR
scanning / always prompted for SSID), but it's kept 5-way to mirror the desktop UI. Don't remove
it without replacing those two behaviors.

## Conventions & gotchas

- Storyboards: `iOS/FlyingCarpet/Base.lproj/Main.storyboard`,
  `macOS/FlyingCarpet/Base.lproj/Main.storyboard`. macOS `peerSwitch` is an
  `NSSegmentedControl`: Android(0), Linux(1), Windows(2), macOS(3), iOS(4) — see "Peer OS
  selection on Apple" above for why it exists. iOS has no peer switch (it uses Bluetooth or the
  QR scanner).
- Duplicate-delivery guards: BLE and PHPicker can deliver twice; password/transfer starts are
  guarded by `transfer.task == nil` on the main queue. Don't remove these.
- `AtomicBool` test-and-set must be atomic under its lock (`testAndSet()`), not a `guard` then a
  separate assignment — that's a TOCTOU race.
- `.gitignore` here covers the Xcode-local noise (`xcuserdata`, `xcshareddata`, `swiftpm`,
  `xcdebugger`, `.DS_Store`). Keep it — those directories embed local usernames and window state.

## History

This code was developed in a separate `FlyingCarpetApple` repository and copied here without its
git history when v10 was released, so `git log` on these files starts at the import commit. The
iOS build submitted to the App Store as **10.0.0 (build 1)** is tagged `ios-10.0.0` in that old
repo. `../docs/apple-receive-throughput.md` is the one investigation doc that came across with
it.
