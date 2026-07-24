# Flying Carpet v10 — Release Plan

Companion to `docs/v10-release-test-plan.md` (what to test) and
`docs/post-v10-maintenance.md` (what's deliberately deferred). This doc holds the **draft
release notes** and the **ship checklist**.

Branch: `shared-network`, both repos. 44 commits, ~8,400 insertions over `main`.
Versions already bumped: Rust core `10.0.0`, Tauri app `10.0.0`, Android `versionName 10.0.0`
/ `versionCode 22`. Apple repo versions still need checking (see checklist).

---

# Draft release notes

## Flying Carpet 10.0

**Flying Carpet 10 adds Shared Network mode and rebuilds the encryption on the Noise
Protocol Framework.**

> ⚠️ **Version 10 is a breaking change.** v10 devices cannot transfer with v9 or earlier —
> you'll get a clear version-mismatch message instead of a hang. **Update every device you
> transfer between.**

### Shared Network mode

Until now Flying Carpet always created its own ad-hoc WiFi hotspot. You can now transfer over
a WiFi **or wired** network that both devices are already on — useful at home or in an office,
and much faster to start when there's a network handy.

- Discovery is authenticated: devices announce themselves on port 3290 with an HMAC keyed
  from the transfer password, so you only ever see peers that hold your password.
- The receiver generates a single-use password and displays it (with a QR code); the sender
  types or scans it. No Bluetooth involved, and the two devices' operating systems don't
  matter — no need to select the peer's OS.
- **This is what finally makes Apple-to-Apple transfers work.** iPhone↔Mac transfers were
  impossible in hotspot mode because neither device can host for the other. Join both to the
  same network — including a hotspot you made manually beforehand — and it just works.
- Interface picker: a labeled dropdown showing each interface's IP, with unusable ones
  hidden, for machines with several NICs.

### Rebuilt encryption

Every transfer, in **both** modes, now runs a
`Noise_NNpsk0_25519_ChaChaPoly_SHA256` handshake (X25519 + ChaCha20-Poly1305 + SHA-256), with
the pre-shared key derived from your password by PBKDF2-HMAC-SHA256 at 600,000 iterations.

- **Forward secrecy.** Recording a transfer and cracking the password later no longer reveals
  the files — each transfer has fresh ephemeral keys.
- **Metadata is encrypted too.** Filenames, file sizes, and the file count used to travel in
  the clear; they're now inside the encrypted channel along with the contents.
- **Tamper-evident.** The plaintext version/mode preamble is bound into the Noise handshake as
  the prologue, so modifying a single byte of it fails the handshake rather than going
  unnoticed. This also closes the door on future downgrade attacks.
- Replaces the previous SHA-256/AES-256-GCM per-chunk scheme; Noise is now the sole cipher.
- Discovery announcements are signed with a key derived from the *stretched* PSK, so no fast
  hash of your password ever goes on the air — every offline guess costs a full 600k-iteration
  PBKDF2.
- Generated passwords are now **10 characters** instead of 8 (~2⁵⁸ instead of ~2⁴⁷),
  foreclosing a precomputed-table attack over the whole password space.
- Full design writeup: `docs/shared-network-crypto.md`. All three implementations (Rust,
  Kotlin, Swift) are held together by shared known-answer test vectors.

Hotspot mode keeps WPA2 underneath, so it's now encrypted twice over.

### macOS ↔ Linux Bluetooth now works

Hotspot transfers between a Mac and a Linux machine previously required manually pairing the
two in System Settings first, and often failed afterward with "Peer removed pairing
information".

Root cause: macOS advertises with a *public* address and dual-mode flags, and BlueZ's bearer
tiebreak prefers classic BR/EDR on a tie — so Linux was connecting over classic Bluetooth,
which macOS serves no GATT over. Linux now bonds over an LE socket first, which pins the
bearer to LE permanently, and keeps the bond for macOS peers so their rotating address stays
resolvable. Pairing also surfaces the 6-digit code in the app for confirmation (real MITM
protection), and declining now aborts the transfer cleanly instead of hanging.

### Other fixes and improvements

**Desktop (Windows/Linux)**

- File selection now comes first everywhere: hotspot joiners pick files and *then* get
  prompted for the password, instead of needing the host's password before the file dialog
  would open.
- The UI recovers if a transfer task panics or is aborted, instead of freezing (#118).
- Shared network mode raises a **single** UAC prompt for its firewall rules on Windows,
  not two.
- Windows Bluetooth: recovers from GATT enumeration failures (`0x8000FFFF`) against
  already-paired iPhones — retries, then re-pairs once within the same transfer, rather than
  failing the run. Advertising is now explicitly stopped after the credential exchange.
- Linux: stale `flyingCarpet_*` NetworkManager connections are pruned at startup and the
  hotspot is torn down on window close (#51).
- The WiFi Direct failure message now points at shared network mode (#115).
- Edge/WebView2 no longer offers previously typed transfer passwords in an autofill dropdown.

**Android**

- Fixed an intermittent iOS→Android hotspot failure ("Empty key" crash or a silent stall)
  caused by a BLE credential-exchange race across two GATT connections.
- Fixed a stuck hotspot flag that made repeat transfers hang with "hotspot already running";
  GATT client and server are now properly closed between transfers.
- Successful shared-network receives no longer end with a spurious
  "Discovery error: StandaloneCoroutine was cancelled".
- Output box auto-scrolls; the transfer log survives screen rotation (it previously could
  overflow the Binder transaction limit and vanish); every line is mirrored to logcat under
  the tag `FlyingCarpet` for easier bug reports.
- Bluetooth pairing that's declined or fails now aborts the transfer instead of waiting
  forever; failed GATT reads no longer propagate empty values as the peer's OS or password.
- Missing Bluetooth *permissions* are distinguished from missing *hardware*, and the switch
  stays usable so you can re-grant (#101).
- Password-prompt dialog buttons are readable in dark mode.

**Security hardening**

- Received filenames are sanitized against path traversal on all five platforms before
  touching the filesystem.
- Header values from the peer (file count, filename length, chunk size) are bounds-checked
  before they're used to size allocations.
- Fixed a Windows WiFi-profile XML injection via the SSID/password fields.
- Dependency updates closing 6 Dependabot advisories, including **CVE-2026-42184** in Tauri
  (`is_local_url()` misclassifying remote URLs as trusted local origins on Windows/Android,
  allowing a remote page to invoke local-only IPC commands). Also CVE-2026-25727 (`time`),
  CVE-2026-25541 (`bytes`), and fixes in `serde_with` and `rand`.

**Send Folder is consistent everywhere**

Sending a folder now recreates that folder inside the destination the receiving device
chose, with the contents inside it, on all five platforms. Previously only macOS did this —
everywhere else the folder's contents were dumped loose into the destination. Sending
individual files is unchanged: they still arrive flat.

This also fixes sending a folder that contains sub-folders from Android to Windows or Linux,
which used to fail outright, and two cases where the desktop app aborted a transfer:
selecting a folder whose top level holds only sub-folders, and dropping two folders (or
files from two different directories) at once.

---

# Ship checklist

## Blockers — must resolve before tagging

- [x] ~~Unify Send Folder across all five platforms~~ — done 2026-07-24; all five now recreate
      the folder on the receiving end. Code landed in both repos; see
      `docs/send-folder-behavior.md`. **Still needs the Tier 6 hardware rows run.**
- [x] ~~Bump Apple passwords to 10 characters~~ — already done: `generatePassword()` returns 10
      (`shared/Transfer.swift`), iOS prompt requires 10, macOS uses `minLength: 10` for shared
      network and `8` for hotspot join (correct — an Android host's WPA2 passphrase isn't ours
      to size).
- [ ] **Run the Tier 6 Send Folder rows on hardware** — behavior changed on four of five
      platforms and only Rust has unit coverage; Kotlin and Swift are verified by hand.
- [ ] **Build the Apple repo.** `shared/Transfer.swift` and the iOS storyboard changed and
      have not been compiled here (no Mac).
- [ ] **Confirm both repos report the same protocol version constant** (wire-compat gate).
- [ ] Finish `docs/v10-release-test-plan.md` — Tiers 2–6 and the release gate.
- [ ] Re-run desktop smoke tests after the Tauri 2.9.5 → 2.11.1 / wry 0.53 → 0.55 bump
      (62-package delta; flagged in `dbdfbb1`).

## Versions and metadata

- [ ] Apple repo: bump iOS and macOS `CFBundleShortVersionString` to 10.0 and increment build
      numbers.
- [ ] Android: confirm `versionCode 22` is greater than what's live on Play and F-Droid.
- [ ] Update the README's sideload APK filename — still says
      `android_FlyingCarpet_9.0.8.apk`.
- [ ] Decide whether `tauri.conf.json`'s stale `icons/icon.icns` entry stays (per CLAUDE.md it
      is intentionally left alone — confirm, don't silently "fix").

## Screenshots and store assets

Every screenshot in `screenshots/` predates the mode switch, the interface dropdown, and the
password-box removal.

- [ ] `screenshots/windows.png` — retake showing the Shared Network / Hotspot mode switch
- [ ] `screenshots/linux.png` — retake; show the interface dropdown with IP labels
- [ ] `screenshots/mac.png` — retake with the current segmented-control layout
- [ ] `screenshots/android.png` — retake (mode switch + Send Folder checkbox)
- [ ] `screenshots/ios.png` — retake, ideally showing shared network mode
- [ ] Take a **receiver-side password + QR** screenshot — it's the single most explanatory new
      screen and there's currently no shot of it anywhere
- [ ] Google Play: updated phone screenshots + feature graphic
- [ ] App Store: updated screenshots for every required device size (iPhone 6.9"/6.5", iPad
      if listed)
- [ ] F-Droid metadata: screenshots and changelog entry
- [ ] Re-record or re-caption the demo video (currently
      https://youtu.be/52Xkrx2BXrg, hotspot-only) — or at minimum add a note in the README
      that it predates shared network mode

## Documentation

- [x] ~~Fix the stale "drag it onto the window" folder instructions~~ — done; each platform's
      help text now describes what it actually supports and states that a sent folder is
      recreated on the receiving end.
- [ ] README: add a Shared Network mode section with the receiver-shows-password flow.
- [ ] README: mention that a sent folder is recreated inside the receiver's destination.
- [ ] README: state plainly that v10 can't talk to v9 and both devices must be updated.
- [ ] Verify README crypto Q&A matches the shipped design (`docs/shared-network-crypto.md`).
- [ ] Check `ARCHITECTURE.md` is accurate for the final state of the branch.

## Build and packaging

- [ ] Windows: `.msi` installer + standalone `FlyingCarpet.exe`, both signed
- [ ] Linux: `.AppImage` + `.deb`
- [ ] macOS: `.dmg`, signed and **notarized**; verify Gatekeeper on a clean machine
- [ ] Homebrew cask update (`brew install flying-carpet`)
- [ ] Android: signed release APK for sideloading + AAB for Play
- [ ] iOS: App Store build submitted (start early — review latency gates the announcement)
- [ ] Verify each artifact launches on a machine that has never had Flying Carpet installed

## Release mechanics

- [ ] Merge `shared-network` → `main` in **both** repos, close together (they must stay
      wire-compatible)
- [ ] Tag `v10.0.0` in both repos
- [ ] Publish the GitHub release with these notes and all binaries (including the Apple ones,
      which ship from this repo)
- [ ] Google Play rollout — consider staged (20% → 100%)
- [ ] F-Droid: confirm the build recipe picks up the new tag
- [ ] Close the issues fixed here: #51, #101, #115, #118

## Post-release

- [ ] Watch for v9↔v10 confusion reports; the mismatch message is the first line of defense
- [ ] Start on `docs/post-v10-maintenance.md` — the `windows` crate 0.44 pin first
- [ ] Re-check the `glib` 0.18.5 advisory at the next Tauri upgrade (Linux/GTK only)
