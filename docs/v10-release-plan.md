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
  same network — including a hotspot you made manually beforehand — and it just works. This
  also unblocks Android↔iOS, which had been broken by the iOS WiFi path (#131).
- **Wired connections are supported** (#124). A desktop on ethernet can transfer with a phone
  on WiFi, as long as they're on the same network — and a machine with no WiFi card at all
  can now use Flying Carpet for the first time (#93).
- Interface picker: a labeled dropdown showing each interface's IP, with unusable ones
  hidden, for machines with several NICs.
- No hotspot means no ad-hoc WiFi connection to drop partway through a large transfer (#130).

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
- **The Windows firewall UAC prompt no longer appears on every transfer** (#129). The check
  for an existing rule passed the rule name to `netsh` with literal quotes around it, so it
  never matched and the rule was re-added every time. Both rules are also added under a
  single elevated command now, so the one-time prompt is one prompt, not two.
- Windows Bluetooth: recovers from GATT enumeration failures (`0x8000FFFF`) against
  already-paired iPhones — retries, then re-pairs once within the same transfer, rather than
  failing the run. Advertising is now explicitly stopped after the credential exchange.
- Linux: stale `flyingCarpet_*` NetworkManager connections are pruned at startup and the
  hotspot is torn down on window close (#51).
- Linux Bluetooth: cached BlueZ entries are purged and discovery is LE-only, so a
  previously-paired device (an audio device, say) can no longer be picked up as the transfer
  peer and then fail with "Could not find service UUID on scanned device" (#106).
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
  the tag `FlyingCarpet`, so a full transfer log can finally be pulled with
  `adb logcat -s FlyingCarpet` for bug reports (#130).
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

The in-app instructions were also wrong about how to send a folder — they told you to drag it
onto the window, which Android and macOS have no handler for, and which is not something a
screen reader user can do (#122). Each platform's help text now describes the control it
actually has, and says what arrives on the other end.

---

# GitHub issues

Nine open issues are resolved or materially addressed by this branch, plus three closed
feature requests that v10 actually delivers. Only four (#51, #101, #115, #118) are cited in
commit messages — the rest were matched by reading the issue against the code, so the
confidence column matters.

| # | Title | Fixed by | Confidence |
|---|---|---|---|
| [#101](https://github.com/spieglt/FlyingCarpet/issues/101) | Bluetooth Button is Greyed Out | `a85d2c4` | **Fixed** |
| [#118](https://github.com/spieglt/FlyingCarpet/issues/118) | Windows receiver freezes | `a85d2c4` | **Fixed** |
| [#124](https://github.com/spieglt/FlyingCarpet/issues/124) | Support wired connection on one device | `d4883ea` | **Fixed** |
| [#129](https://github.com/spieglt/FlyingCarpet/issues/129) | Firewall UAC prompt every time | `25050cb`, `04da20b` | **Fixed** |
| [#131](https://github.com/spieglt/FlyingCarpet/issues/131) | Android to iOS not working | shared network mode | **Fixed** (you already told the thread this was the plan) |
| [#106](https://github.com/spieglt/FlyingCarpet/issues/106) | Bluer: GATT services not resolved | `967ed6b` | Likely — ask reporter to retest |
| [#51](https://github.com/spieglt/FlyingCarpet/issues/51) | Does not clean up after itself (Debian) | `a85d2c4` | **Partial** — no uninstall purge |
| [#115](https://github.com/spieglt/FlyingCarpet/issues/115) | Failed to start WiFi Direct AP | `a85d2c4` + shared network | **Worked around**, not fixed |
| [#130](https://github.com/spieglt/FlyingCarpet/issues/130) | WiFi drops midway on large transfers | shared network, `478ee4d` | **Partial** + diagnosis unblocked |

Closed feature requests v10 delivers — worth a courtesy follow-up, since the requesters never
got an answer: [#93](https://github.com/spieglt/FlyingCarpet/issues/93) (detect same network,
skip the hotspot), [#61](https://github.com/spieglt/FlyingCarpet/issues/61) (static network
option), [#122](https://github.com/spieglt/FlyingCarpet/issues/122) (folder-select button for
screen reader users).

## Notes on the non-obvious matches

**#129 was a real bug, not a design choice.** `check_for_firewall_rule` built its query as
`format!("name=\"{}\"", file_name)` and passed it to `process::Command` as one argument, so
netsh searched for a rule whose name *literally contained quote characters*. It never
matched, the app concluded the rule was missing, and re-added it — raising UAC on every
single transfer. Fixed in `25050cb`; `04da20b` then collapsed the two `netsh` invocations
into one elevated `cmd.exe` so the first run costs one prompt rather than two.

**#106's log identifies its own cause.** The device it connected to advertises service UUIDs
`1101`/`110b`/`110c`/`110d`/`110e`/`111e` — Serial Port, Audio Sink, AV Remote Control,
Handsfree. That's a classic Bluetooth audio device, not the reporter's iPhone. `967ed6b`
records that bluer's `discover_devices()` pre-seeds results with every device BlueZ already
knows, ahead of the discovery filter, so a previously-paired peripheral could be returned as
the peer. The same commit purges those cached entries, scans LE-only, and retries/rescans on
enumeration failure.

**#115 is not fixed and shouldn't be closed as such.** "Failed to start WiFi Direct AP" is a
card/driver limitation. v10 only offers a route that avoids it.

## Adjacent open issues — do NOT close with this release

- [#109](https://github.com/spieglt/FlyingCarpet/issues/109) Resume after lost connection —
  shared network mode makes drops rarer, but there's still no resume.
- [#58](https://github.com/spieglt/FlyingCarpet/issues/58) Log clearing / newest-first
  ordering / VPN warning — Android got autoscroll and logcat mirroring, but none of the three
  asks.
- [#133](https://github.com/spieglt/FlyingCarpet/issues/133) Tighter UI + user-chosen password
  — the UI half is untouched. **The password half is a deliberate no:** single-use CSPRNG
  passwords are what make the "an offline crack is worthless" argument hold
  (`docs/shared-network-crypto.md` §7). Worth answering kindly and explaining why, rather than
  leaving it open indefinitely.
- [#134](https://github.com/spieglt/FlyingCarpet/issues/134) send/receive text,
  [#81](https://github.com/spieglt/FlyingCarpet/issues/81) share sheet,
  [#75](https://github.com/spieglt/FlyingCarpet/issues/75) adaptive icon,
  [#62](https://github.com/spieglt/FlyingCarpet/issues/62) Android 5GHz hotspot — untouched.

---

# Draft issue responses

Copy-paste ready. Post **after** the release is live so the download links work. Where a
response asks the reporter a question, that's deliberate — several of these are worth
confirming before closing.

### #118 — Windows receiver freezes

> Should be fixed in v10.
>
> The transfer task could panic or be aborted without ever re-enabling the UI, which left the
> window looking frozen — the app was still running, but every control stayed disabled. v10
> emits the re-enable from a drop guard so the UI recovers even when the transfer task dies
> unexpectedly, and the setup paths that used to panic now print an error instead.
>
> One thing that would help me confirm: when it freezes, is the window unresponsive to clicks
> entirely, or are the controls just greyed out and unclickable? Those are two different
> problems and it's the second one I've fixed. If it's the first, and especially in the "kept
> open for a long time" case rather than "after completing a transfer", I'd like to keep this
> open.

### #101 — Bluetooth button greyed out

> Fixed in v10.
>
> The app was treating "Bluetooth permissions haven't been granted yet" and "this device has
> no Bluetooth hardware" as the same state, and showed "Device can't use Bluetooth" for both.
> That's also why the switch came to life after you tapped "Select file" — that's the point
> where the permission request actually fired.
>
> v10 tells the two apart, keeps the switch tappable so it can re-request the permission, and
> re-checks when the app returns to the foreground, so granting it in Settings now takes
> effect without restarting the app.

### #129 — Firewall UAC prompt every time

> Fixed in v10 — and it was a bug, not a design decision. The rule was only ever meant to be
> added once.
>
> The check for "do I already have a firewall rule?" passed the rule name to `netsh` with
> literal quote characters around it, so `netsh` went looking for a rule whose name actually
> contained quotes. It never matched, so the app concluded the rule was missing and re-added
> it — every transfer, hence the prompt every transfer. v10 passes the name unquoted and finds
> the existing rule.
>
> One caveat: v10 adds a second (UDP) rule for shared-network discovery, so the first run
> after upgrading will prompt once to add it. That's a single prompt covering both rules now
> rather than one each. After that it should be silent.

### #124 — Wired connection on one device

> This works in v10.
>
> Shared Network mode transfers over a network both devices are already on, and it supports
> wired interfaces explicitly — so your exact setup (Windows on ethernet, Android on WiFi,
> same network) is supported. The interface picker lists wired adapters alongside wireless
> ones, labelled with their IP.
>
> The "not connected to WiFi" error you hit came from hotspot mode needing a WiFi card to host
> the hotspot with. In shared network mode nothing is hosted, so that requirement is gone.

### #131 — Android to iOS not working

> v10 is the fix for this, and it's close to release.
>
> As I mentioned above, the iOS WiFi side was broken and the fix is the shared network mode
> I've been building. Join both devices to the same WiFi network and start the transfer: the
> receiving device shows a password (with a QR code) that you enter on the sender. No hotspot,
> no Bluetooth, and the two devices' operating systems no longer need to be selected.
>
> @B5-SA this should cover Linux Mint ↔ iOS too. v10 separately fixes Linux failing to
> enumerate GATT services over Bluetooth, which is likely what you hit on the pairing side —
> see #106.

### #106 — Bluer: GATT services have not been resolved

> I think v10 fixes this, and your log is what convinced me — thank you for pasting the whole
> thing.
>
> Look at the device it picked up: service UUIDs `1101`, `110b`, `110c`, `110d`, `110e`,
> `111e` — Serial Port, Audio Sink, A/V Remote Control, Handsfree. That's a classic Bluetooth
> audio device, not your iPhone. bluer's `discover_devices()` pre-seeds its results with every
> device BlueZ already knows about, *before* the discovery filter applies, so a
> previously-paired peripheral could come back as the "peer" — and naturally it has no Flying
> Carpet GATT service on it.
>
> v10 scans LE-only, purges unpaired cached BlueZ entries carrying our service UUID before
> discovery starts, retries service enumeration, and removes + rescans once if it still fails.
> Would you be willing to retest once v10 is out?

### #51 — Does not clean up after itself (Debian)

> Partly addressed in v10, and I'd rather be straight about which part.
>
> **Fixed:** Flying Carpet no longer accumulates `flyingCarpet_*` connections as you use it.
> v10 prunes stale ones at startup and tears the hotspot down when the window closes, so a
> crashed or force-quit transfer doesn't leave one behind indefinitely.
>
> **Not fixed:** there's still no uninstall-time purge. The AppImage has no uninstall hook to
> attach one to, and the `.deb` would need a `postrm` script.
>
> Before I add a `--purge` flag people would have to know exists: would the startup pruning
> have been enough for your case, or were the leftovers specifically a problem *after* you'd
> uninstalled? If it's the latter I'll do the `postrm` script for the `.deb` at least.

### #115 — Failed to start WiFi Direct AP

> v10 gives you a way around this, though I want to be clear it isn't a fix for the underlying
> problem.
>
> "Failed to start WiFi Direct AP" means the WiFi card or its driver won't host a software
> access point. That's a hardware/driver limitation Flying Carpet can't work around, which is
> why the message says what it says.
>
> What v10 adds is Shared Network mode: if both devices are already on the same WiFi or wired
> network, no hotspot is created and the WiFi Direct path is never used. The error message now
> points there too. Worth a try when v10 lands.

### #130 — Drops the WiFi midway during large transfers

> Two things in v10 for this.
>
> The direct one is Shared Network mode: if both devices are already on the same network, no
> hotspot is created, so there's no ad-hoc WiFi connection to drop midway. For multi-GB
> transfers from Android that's the path I'd recommend.
>
> The other is the thing you actually asked for — "I can't find any way to log/share error out
> of the box as an end user." Fair, and fixed. Every line the Android app prints now also goes
> to logcat under the tag `FlyingCarpet`, so you can pull a full transfer log with
> `adb logcat -s FlyingCarpet`. The log also survives screen rotation now; it could previously
> be wiped mid-transfer.
>
> v10 also fixes an intermittent Bluetooth credential-exchange race on Android, which may well
> be the "hit and miss" half of what you were seeing.

### #93 (closed) — Detect same network, avoid the hotspot

> Following up a year and a half later: this is shipping in v10, more or less exactly as you
> described it. If both devices are on the same network there's no hotspot at all — and yes,
> that makes transfers to desktops without WiFi cards work, since wired interfaces are
> supported.

### #61 (closed) — Static network option

> Following up: v10's shared network mode should remove the need for this. Instead of a fixed
> SSID and password per peer, if both devices are already on a network there's no hotspot to
> name — the receiving device shows a one-time password (with a QR code) and that's the only
> thing to type. The long SSID + password entry for Mac ↔ Android goes away entirely.

### #122 (closed) — Folder-select button for screen reader users

> Following up, because v10 improves this and you deserved a better answer at the time.
>
> There's a "Send Folder" checkbox now, so sending a folder no longer requires drag-and-drop.
> More to the point, the instructions text still said "To send a folder, drag it onto the
> window" — which was exactly the wrong thing to tell a screen reader user, and it stayed that
> way far too long. It now describes the checkbox instead.
>
> v10 also makes a sent folder arrive *as a folder* on the receiving device on every platform.
> Previously most platforms scattered the contents loose into the destination folder, which I
> imagine was its own kind of unpleasant to sort out without sight.

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
- [x] **Build the Apple repo.** `shared/Transfer.swift` and the iOS storyboard changed and
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
- [ ] Post the drafted issue responses (see "GitHub issues" above) once the release is live
      and the download links work
- [ ] Close #101, #118, #124, #129, #131 outright
- [ ] Leave #51, #106, #115, #130 **open** pending reporter confirmation — each response asks
      a question, and #115 in particular is worked around rather than fixed
- [ ] Comment on closed #93, #61, #122; leave them closed

## Post-release

- [ ] Watch for v9↔v10 confusion reports; the mismatch message is the first line of defense
- [ ] Start on `docs/post-v10-maintenance.md` — the `windows` crate 0.44 pin first
- [ ] Re-check the `glib` 0.18.5 advisory at the next Tauri upgrade (Linux/GTK only)
