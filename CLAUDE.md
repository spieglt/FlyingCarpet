# Flying Carpet

Encrypted, no-internet file transfer between **Android, iOS, Linux, macOS, and Windows**, over either an ad-hoc Wi-Fi hotspot (one device hosts) or a **shared network** both devices are already on. Two devices, Wi-Fi (or ethernet), optionally Bluetooth. Port **3290** throughout.

## Two repos, one wire protocol (critical)

Flying Carpet is split across two repositories that **must stay wire-compatible** — a change to the on-the-wire protocol in one is a breaking change unless mirrored in the other:

- **This repo** (`~/Desktop/FlyingCarpet`) — Rust core (`core/`), Tauri desktop app (`Flying Carpet/`), Android app (`Android/FlyingCarpet/`, Kotlin).
- **`~/Desktop/FlyingCarpetApple`** — iOS/macOS apps (Swift); shared protocol code in `shared/`. Not public. Build only on a Mac.

When touching the protocol (discovery bytes, version/mode preamble, Noise handshake, framing), update **all three implementations** (Rust, Kotlin, Swift) and their cross-platform known-answer tests together. The Rust core is the reference the other two are tested against.

### Where code lives vs. where binaries ship (don't conflate these)

- **Code in this repo builds for Windows and Linux only.** `core/src/` has just `windows/` and `linux/`; `lib.rs` cfg-selects `network`/`bluetooth` on those two `target_os` values, and there is no `target_os = "macos"` anywhere in `core/`. A Tauri/`wry`/`webkit2gtk` change here therefore affects **two** desktop platforms, not three.
- **But macOS and iOS binaries are released from this repo's Releases page**, even though their source is in `FlyingCarpetApple`. So `README.md` documenting a macOS `.dmg` download is **correct, not stale** — don't "fix" it, and don't infer from it that the Rust code targets macOS.
- `tauri.conf.json` still lists `icons/icon.icns` — genuinely stale, intentionally left alone. Not evidence of macOS support either.

## Layout

- `core/` — Rust core crate `flying-carpet-core` (v10). Platform-split: `core/src/{windows,linux}/` for network/bluetooth/peripheral/central; the `bluetooth` module is `cfg`-selected per-OS in `lib.rs`. Key files: `lib.rs` (`start_transfer` entry point), `discovery.rs`, `noise.rs`, `sending.rs`/`receiving.rs`.
- `Flying Carpet/` — Tauri desktop app. Rust backend in `Flying Carpet/src-tauri/` (workspace member), JS/HTML frontend in `Flying Carpet/src/` (`main.js`, `index.html`). **Note the space in the directory name** — quote it in shell commands.
- `Android/FlyingCarpet/` — Android app (Kotlin). Noise/discovery ports in `app/src/main/java/dev/spiegl/flyingcarpet/`.
- `docs/` — design docs (see below). `ARCHITECTURE.md` — connection role model.

## Build & test

Rust is a Cargo **workspace** (`core` + `Flying Carpet/src-tauri`):

- `cargo test` — run all Rust tests (includes the Noise/discovery known-answer vectors in `core/src/noise.rs` and `core/src/discovery.rs`). `cargo build` to compile.
- Desktop app: `cargo tauri dev` (run) / `cargo tauri build` (release). Needs the Tauri CLI and the Linux deps listed in `README.md`.
- Android: from `Android/FlyingCarpet/`, `./gradlew assembleDebug` / `./gradlew test`. **Set `JAVA_HOME` to the Android Studio JBR** (the bundled JDK) or Gradle fails.

## Architecture & design docs — read before changing these areas

- **`ARCHITECTURE.md`** — the four independent role axes (transfer direction / BLE peripheral-central / hotspot host-joiner / TCP server-client) and how they map per platform pair. Read before touching connection setup, hosting logic, or BLE.
- **`docs/shared-network-crypto.md`** — the full v10 cryptographic design (the normative reference for the handshake bytes). Read before touching anything crypto, discovery-auth, or the record/framing layer.
- **`docs/bluetooth-field-guide.md`** — **read before touching any BLE code on any platform.** The four independent axes (advertising / scanning / bonding / GATT services), seven hard-won laws, a per-platform matrix, and a symptom→cause playbook. Bluetooth bugs here are subtle, intermittent, and platform-asymmetric; several have been re-derived from scratch more than once. Chronological investigation logs: `docs/windows-ble-gatt-0x8000ffff.md`, `docs/ble-bond-asymmetries.md`.

## Load-bearing invariants (don't "simplify" these)

- **v10 = Noise.** Every transfer (both modes) runs a `Noise_NNpsk0_25519_ChaChaPoly_SHA256` handshake; the PSK is `PBKDF2-HMAC-SHA256(password, salt="Flying Carpet v10 shared network PSK", 600_000)`. Noise is the **sole** cipher — the old inner per-chunk AES is gone. v10 is a clean break; v9 peers are rejected. If v10 ships before a later Noise wire change, that change must bump to v11.
- **Preamble → prologue binding.** Version/mode are negotiated in a plaintext preamble, then every preamble byte is bound into the Noise prologue. Both platforms of any pair must build the prologue identically (`build_prologue`/`buildPrologue`). Cross-platform KATs guard this — keep them in sync (Rust `core/src/noise.rs`, Kotlin `NoiseUnitTest`, Swift `NoiseTests`; discovery vector: `core/src/discovery.rs` `test_cross_platform_vector` == Android `DiscoveryUnitTest.kt`).
- **Passwords: single-use + CSPRNG.** The receiver mints a fresh random password per transfer and displays it; never reuse, never user-chosen, never "remember." The entire "offline crack is worthless" security argument depends on this (see the crypto doc §7). The discovery HMAC key is derived from the stretched PSK, so no fast hash of the password goes on the wire.
- **Bluetooth is hotspot-only.** Shared network mode exchanges the password manually (display + type/QR); do **not** re-add BLE to shared mode. Apple-to-Apple can't pair iPhone↔Mac over BLE by design, which is exactly the pair that would need it. Rationale is recorded in `ARCHITECTURE.md` ("Bluetooth + Shared Network Mode").
- **Receiver is the anchor.** In both modes the receiver generates the password and is the TCP server (Noise responder); the sender is the TCP client (Noise initiator).

## Conventions & gotchas

- Android: keep `res/layout/` and `res/layout-land/` in sync when changing the UI.
- `core/Cargo.toml` is pinned to **LF** line endings (`.gitattributes`); don't let an editor rewrite it to CRLF.
- Header-value bounds (file count / filename length / chunk size sanity checks) and filename sanitization apply to values read from the **Noise-decrypted** stream, not the raw socket.
