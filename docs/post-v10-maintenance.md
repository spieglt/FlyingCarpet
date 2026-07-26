# Post-v10 Maintenance Backlog

Work deliberately deferred until **after v10 ships**, plus findings from post-release field
reports. None of it is blocking. The housekeeping items touch code that the v10 release
testing already covers, so doing them mid-release would invalidate hardware testing for no
benefit; revisit once `docs/v10-release-test-plan.md` is signed off.

---

## 1. Consolidate the duplicate `windows` crate versions

The dependency tree currently carries **three** semver-incompatible versions of the
`windows` crate (windows-rs):

| Version | Pulled in by | Ours to control? |
|---|---|---|
| **0.44.0** | `wifidirect-legacy-ap 0.4.0` → `flying-carpet-core` | **Yes** — our own crate (`github.com/spieglt/wifidirect-legacy-ap`), ~193 LOC, last published Feb 2023 |
| **0.58.0** | `flying-carpet-core` directly (`core/Cargo.toml`) | **Yes** |
| **0.61.3** | Tauri stack (`tao`, `tauri-runtime`, `tauri-runtime-wry`, `webview2-com`) | No — moves on Tauri's release schedule |

**Why this is not urgent.** Multiple semver-incompatible versions are normal in Rust and
cargo supports them by design. The only real cost is build time and some binary size (the
`windows` crate is heavily feature-gated, so far less than the version count suggests).
They only actually *break* something when a `windows` type has to cross a crate boundary —
e.g. an attempt to reach Tauri's `ICoreWebView2` from our code failed to type-check until a
matching `webview2-com`/`windows` pair was added as a direct dependency. No current feature
needs that.

**Why it's worth doing eventually.** The `0.44.0` pin dates to February 2023 and will
eventually block something. It is also the easiest to fix, since we own the crate.

**Suggested order (lowest risk first):**

1. **Bump `wifidirect-legacy-ap`** to a modern `windows` and publish 0.5.0. Separate repo,
   ~193 lines, isolated from the app — easy to verify on its own.
2. **Bump `core` from 0.58 → 0.61** and fix the call sites. This is the risky half:
   windows-rs has breaking changes across those versions (error/`Result` handling,
   `BOOL` vs `bool`, the `Param`/`IntoParam` traits, the `core::Interface` reorganization),
   and `core/src/windows/{bluetooth,peripheral,central,network}.rs` lean on it heavily for
   GATT and hotspot work. Budget time to re-run the Windows BLE + hotspot hardware tests
   afterward.

**Caveat — don't chase Tauri.** "Match whatever version Tauri uses" is a treadmill: Tauri
will move to 0.62+ and we will diverge again. The durable goal is **retiring the 0.44 pin**;
landing on the same version as Tauri is a pleasant side effect, not the objective.

---

## 2. Mobile transfers die when the app is backgrounded or the screen sleeps

Reported by users and reproduced informally. **Neither mobile app does anything to keep
running when it loses the foreground** — this is not a bug in the transfer code, it's a
missing platform integration on both sides. Confirmed by audit, 2026-07-26:

- **Android** — `AndroidManifest.xml` declares no `<service>` at all, no `FOREGROUND_SERVICE`
  permission, no `WakeLock`, no `WifiLock`, and no `FLAG_KEEP_SCREEN_ON`. The only lock
  anywhere is the `MulticastLock` in `Discovery.kt:183`. Transfers run in bare
  `GlobalScope.launch` coroutines off `MainViewModel` (`MainViewModel.kt:366`, `:566`,
  `:906`), with UI callbacks bound to one Activity instance (`MainActivity.kt:240-244`).
- **iOS** — `Apple/iOS/FlyingCarpet/Info.plist` has no `UIBackgroundModes` key.
  `SceneDelegate.swift:44` `sceneDidEnterBackground` is still the empty Xcode template. No
  `beginBackgroundTask`, no `isIdleTimerDisabled` anywhere in `Apple/`.

**Screen sleep and app-switch are the same event on both platforms.** Display timeout stops
the Activity on Android and backgrounds the scene on iOS. Any fix has to cover both, and the
screen-timeout case is almost certainly the bulk of the reports: user starts a transfer, puts
the phone down, the display sleeps, the peer sees a dead socket.

### iOS: structurally cannot continue in the background

There is no way around this, and it should be treated as a constraint to communicate rather
than a bug to fix. Apple DTS on this exact scenario ([forums/thread/715118](https://developer.apple.com/forums/thread/715118)):

> "The thing to keep in mind with networking in iOS is that the background/foreground state
> isn't key, but rather the **suspended/running state**. Networking works just fine as long as
> your process is running. Once it's suspended, everything just stops." … "When the app on
> Device2 gets suspended, its TCP connections will likely be closed immediately." … "My
> general advice is that, when your app moves to the background you should shut down your
> networking, resuming it when you come back into the foreground."

No background mode covers a raw peer-to-peer TCP transfer. The list is audio, location, voip,
external-accessory, bluetooth-central, bluetooth-peripheral, fetch, processing. `bluetooth-central`
would only keep the BLE credential exchange alive, never the Wi-Fi transfer; `NSURLSession`
background transfers only work against an HTTP server. In hotspot mode the
`NEHotspotConfiguration` association to a no-internet network may also be dropped once
suspended, so even resuming won't reliably find the peer.

What can be done:

1. **`UIApplication.shared.isIdleTimerDisabled = true` for the duration of a transfer.** The
   single highest-value change on iOS — it eliminates the dominant cause outright. Roughly
   five lines in `toggleUI(transferRunning:)`, set and cleared symmetrically with the rest of
   the transfer UI state.
2. **`beginBackgroundTask` around the transfer** buys ~30 seconds (iOS 13 cut the
   from-foreground grant to about that). Enough to survive a glanced-at notification or a
   quick app switch and back; nowhere near enough for a real transfer.
3. **Fail loudly instead of hanging.** `sceneDidEnterBackground` currently does nothing, so
   the user watches a frozen progress bar and then gets a generic socket error. Wire it to the
   running `Transfer` and emit something explicit — "Flying Carpet was moved to the
   background; iOS suspends apps and cannot continue transfers there." Turns a mystery bug
   report into a comprehensible limitation.

### Android: fixable, and worth fixing

Android doesn't force-suspend the process, it *kills* it. Once no Activity is visible the
process has no foreground component and drops to a **cached** process, eligible for kill at
any moment under memory pressure. That explains the intermittent, device-dependent character
of the reports. On top of that, screen-off puts the Wi-Fi radio into power save and lets the
SoC suspend.

**Checked and found false:** AOSP's current `WifiNetworkFactory` validates foreground status
only at *request* time (`isRequestFromForegroundAppOrService` in `acceptRequest()`); there is
no continuous importance monitoring that revokes an established `WifiNetworkSpecifier`
connection when the app backgrounds. So the joined hotspot is not proactively torn down — the
process dying is what kills it. Don't waste time chasing a framework teardown that isn't there.

Four changes, in descending value-per-effort:

1. **`FLAG_KEEP_SCREEN_ON` while a transfer runs.** One line, alongside the existing
   orientation lock at `MainActivity.kt:317`. Same reasoning as iOS: mostly makes the problem
   not happen.
2. **A foreground service.** The actual fix. Type **`connectedDevice`** is the right one, and
   its runtime prerequisite is already satisfied — it accepts `CHANGE_NETWORK_STATE`,
   `CHANGE_WIFI_STATE`, or `CHANGE_WIFI_MULTICAST_STATE`, all three of which the manifest
   already declares. Needs `FOREGROUND_SERVICE` + `FOREGROUND_SERVICE_CONNECTED_DEVICE`, plus
   `POST_NOTIFICATIONS` for the notification on API 33+. Declaring a type is mandatory at
   `targetSdk = 37` anyway. Starting it from the Start button is a legal foreground start, and
   the notification doubles as progress display and a cancel action. (`dataSync` also matches
   the description but carries Android 15's extra restrictions; `connectedDevice` is cleaner.)
3. **A `PARTIAL_WAKE_LOCK`** for the transfer's duration. A foreground service does **not**
   keep the CPU awake — a commonly missed point. With the screen off the SoC suspends and the
   transfer threads stop.
4. **A `WifiLock` in `WIFI_MODE_FULL_HIGH_PERF`.** Documented as keeping Wi-Fi at high
   performance "even when the device screen is off." **Do not use
   `WIFI_MODE_FULL_LOW_LATENCY` here** — AOSP documents it as activating only when the app
   "is running in the foreground" *and* "the screen is on," precisely the case that needs no
   help. HIGH_PERF is deprecated but is the mode that covers screen-off. This matters for
   shared-network discovery too: the `MulticastLock` stops the driver filtering multicast, but
   does nothing about the radio entering power save.

**Sequencing.** Items 1, 3, and 4 are small and independently shippable; do them first. Item 2
is a real refactor, not a drop-in: `MainViewModel` currently owns the sockets *and* calls back
into a specific Activity for `displayQrCode`, `promptForPassword`, and `cleanUpUi`. Transfer
state has to move into the service with the UI observing it, rather than the transfer holding
an Activity reference.

---

## 3. Android share sheet

Long-standing TODO at `MainActivity.kt:795`. Sending a file would start from the sharing app
rather than from Flying Carpet.

**Manifest.** Add an intent filter to `MainActivity` for `ACTION_SEND` and
`ACTION_SEND_MULTIPLE` with `category.DEFAULT` and `mimeType="*/*"`. Set
`android:launchMode="singleTop"` and override `onNewIntent` — otherwise sharing into the app
while a transfer is running spawns a **second** MainActivity with a fresh ViewModel while the
first still holds the hotspot and sockets.

**The refactor that makes it work.** `getFilePicker()` (`MainActivity.kt:66-108`) has the
entire "we have files, now proceed" sequence inlined in its result callback: build
`DocumentFile`s, open `InputStream`s, then either BLE advertise/scan or `connectToPeer()`.
Extract that body into something like `stageFilesForSending(uris: List<Uri>)` so the picker
and the share intent share one path. Read the URIs from `EXTRA_STREAM` via
`IntentCompat.getParcelableExtra` / `getParcelableArrayListExtra` — the untyped overloads are
deprecated at API 33+.

**UX: don't auto-start.** In hotspot mode the user still has to choose peer OS and connection
mode. Cleanest flow: the share intent lands, files are staged, the mode toggle flips to Send,
the folder checkbox hides, the Start button reads "Start" instead of "Select Files", and the
Start handler skips the picker when staged URIs exist. Folder sends don't apply — the share
sheet hands over files, so `sendFolder` stays false and a multi-file share maps onto the
existing multi-file path with empty `filePaths`.

**Two real gotchas:**

- **Shared URIs are not `ACTION_OPEN_DOCUMENT` URIs.** `DocumentFile.fromSingleUri` mostly
  works on them by accident — `DocumentsContract.Document.COLUMN_DISPLAY_NAME` and
  `COLUMN_SIZE` happen to be the same column strings as `OpenableColumns.DISPLAY_NAME`/`SIZE`
  — but it isn't guaranteed, and `file://` URIs (still shared by some apps) fail outright.
  `sendFile` depends on `file.name` (`Send.kt:69`, which throws "Could not get filename" on
  null) and `file.length()` (`Send.kt:13`, `:20`, `:33`, `:81`). Worth a small
  name/size/openStream abstraction, or at minimum a `file://` → `DocumentFile.fromFile` branch.
- **The share grant is Activity-scoped** and revoked when the Activity finishes; unlike an
  OPEN_DOCUMENT grant it cannot be persisted. Opening all the `InputStream`s eagerly (which
  the current code already does) covers most of it, but `hashFile` (`Utilities.kt:114-116`)
  *reopens* the URI mid-transfer during the resume/skip check. If the Activity is gone by
  then, that fails — another argument for the foreground service in §2.

iOS is a different design entirely — see §4.

---

## 4. iOS share menu

Worth doing, but **do not assume it mirrors the Android design.** Two iOS-specific
restrictions rule out the obvious approach, and between them they mean there is no single
mechanism that both handles multiple files *and* lands the user in the app. Verified
2026-07-26.

### Restriction 1: a Share Extension cannot open the containing app

The natural design — extension stages the files, then opens Flying Carpet to run the transfer
— is **not supported and is an App Store risk.** From Apple's Frameworks Engineer on
[forums/thread/773342](https://developer.apple.com/forums/thread/773342):

> "There's no supported way for you to launch your app directly from App Extensions, except
> Today and Widgets (which requires `OpenURLIntent` and is available to processes that can use
> App Intents), with the APIs currently available."

`NSExtensionContext.open(_:)` is documented for Today extensions only, and the responder-chain
walk to reach `UIApplication.openURL` is the exact Objective-C runtime bypass Apple calls out
as unsupported. Apple offers no sanctioned alternative — their suggestion is a local
notification, or a Feedback Assistant enhancement request. **Don't build on this.**

### Restriction 2: the document-types route is effectively single-file

Declaring `CFBundleDocumentTypes` + `LSSupportsOpeningDocumentsInPlace` puts a "Copy to Flying
Carpet" action in the share sheet, and that route *does* launch the app, delivering the file
through `scene(_:openURLContexts:)`. No extension, no app group, no new target — much cheaper
than a Share Extension. But open-URL requests are atomic and don't carry multiple URLs; iOS
delivers only the first file even when several are shared. Multi-select in Photos may not
offer the app at all.

### The resulting shape

| | Multi-file | Opens the app | Placement | Cost |
|---|---|---|---|---|
| **Document types** (`CFBundleDocumentTypes`) | No — first file only | **Yes** | Lower "actions" row | Info.plist only |
| **Share Extension** | **Yes** | No — user must switch manually | Top app row | New target, app group, provisioning, App Store resubmit |

**Suggested order.** Start with **document types** — it is an Info.plist change, it covers the
single-file case (share one video to Flying Carpet, which is likely the common one), and it
actually opens the app. Add the Share Extension later for multi-select, accepting that it can
only stage files and show "N files ready — open Flying Carpet to send," leaving the user to
switch apps.

### If/when the Share Extension is built

- **Stage into an App Group container, never the Documents directory.** `emptyDocsDir()`
  (`iOS/FlyingCarpet/ViewController.swift:579`) sweeps `.documentDirectory` wholesale and runs
  both at launch and before every transfer (`:65`, `:455`) — anything the extension dropped
  there would be deleted before it could be sent. Files must land in the shared container and
  be adopted deliberately into `transfer.fileList` on next foreground.
- **Sweep the container too.** The extension has no way to know whether the user ever opened
  the app, so staged files accumulate. Extend the `emptyDocsDir()` discipline to the group
  container.
- **Don't reuse the Live Photo staging pattern in the extension.** The PHPicker path reads a
  whole asset resource into an in-memory `NSMutableData`
  (`iOS/FlyingCarpet/ViewController.swift:~161-183`); extensions run under a far tighter memory
  budget than the host app. Use `loadFileRepresentation` + a filesystem copy, which streams.
- **Scope `NSExtensionActivationRule` deliberately.** `TRUEPREDICATE` offers Flying Carpet for
  text and URLs, which it can't send. Use the
  `NSExtensionActivationSupportsFileWithMaxCount` / `ImageWithMaxCount` / `MovieWithMaxCount`
  keys with generous counts.
- **Provisioning is the hidden cost.** A new bundle ID
  (`dev.spiegl.FlyingCarpet.ShareExtension`), an App Group entitlement on both targets, and its
  own profile — on top of `DEVELOPMENT_TEAM` already being deliberately blank in
  `project.pbxproj` (see `Apple/CLAUDE.md`). The iOS app currently declares no URL scheme and
  no app group, and shipped as 10.0.0 build 1, so this means a fresh App Store submission.

macOS could take the same treatment via a Share Extension, but Macs have drag-and-drop and
`NSSharingService` already; lower priority.

---

## 5. Dependency housekeeping notes

The 2026-07-23 Dependabot sweep is done and its resolved-alert detail has been dropped from
this doc. What's still worth carrying:

- **`glib` (alert #26) remains open, blocked upstream.** Reached via `atk 0.18.2` →
  `gtk 0.18.2` → `muda` → `tauri`. Even Tauri 2.11.1 still pins the gtk-rs **0.18** family and
  the fix needs 0.20. **Linux/GTK-only**, so Windows and macOS builds are unaffected. Re-check
  whenever Tauri is next upgraded.
- **`rand` 0.7.3** is also in the lock and in an affected range, but it comes from
  `phf_generator` as a **build-time** dependency — not in the runtime graph, not independently
  updatable. Expect it to keep showing up.
- **The desktop frontend's JS/CSS is not covered by Dependabot.** `Flying Carpet/src/deps/`
  (`bootstrap.min.css`, `qrcode.js`) is vendored with no `package.json`, so it must be
  refreshed by hand.
- **Toolchain is now rustc 1.97.1** (up from 1.85.0). If `rustup update stable` fails on the
  deprecated `rls-preview` component, remove it with
  `rustup component remove --toolchain stable rls-preview` and retry.
- **rust-analyzer proc-macro crashes are version skew, not code.** `all proc-macro server
  workers have exited` on every `#[tauri::command]` and `#[derive(...)]` was rustc drifting
  ~17 months behind the auto-updating rust-analyzer in the VS Code extension, breaking the
  proc-macro bridge ABI. Fixed by the toolchain upgrade; restart the server afterward. If it
  recurs, suspect toolchain/rust-analyzer skew before suspecting the code.
