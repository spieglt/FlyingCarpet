# Post-v10 Maintenance Backlog

Housekeeping deliberately deferred until **after v10 ships**. None of it is user-visible or
blocking; all of it touches code that the v10 release testing already covers, so doing it
mid-release would invalidate hardware testing for no benefit. Revisit once
`docs/v10-release-test-plan.md` is signed off.

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

## 2. Dependabot alerts — resolved 2026-07-23

All 7 open alerts were **Cargo** (none on Gradle). Note the desktop frontend's JS/CSS is
vendored in `Flying Carpet/src/deps/` (`bootstrap.min.css`, `qrcode.js`) with no
`package.json`, so it is **not** covered by Dependabot and must be refreshed by hand.

**Toolchain upgraded as part of this: rustc 1.85.0 → 1.97.1.** Two alerts (`time`,
`serde_with`) were blocked purely by the Rust version and could not be fixed without it.
(`rustup update stable` first failed on the deprecated `rls-preview` component; remove it
with `rustup component remove --toolchain stable rls-preview`, then retry.)

| # | Package | Change | Status |
|---|---|---|---|
| 30 | `bytes` | 1.11.0 → **1.12.1** | ✅ Fixed |
| 34 | `rand` (0.8 line) | 0.8.5 → **0.8.7** | ✅ Fixed (direct dep in `core/Cargo.toml`) |
| 35 | `rand` (0.9 line) | 0.9.2 → **0.9.5** | ✅ Fixed |
| 31 | `time` | 0.3.44 → **0.3.54** | ✅ Fixed (needed the toolchain bump) |
| 37 | `serde_with` | 3.16.1 → **3.21.0** | ✅ Fixed (needed the toolchain bump) |
| 36 | `tauri` | 2.9.5 → **2.11.1** | ✅ Fixed — required `--precise`; plain `-p` update can't bump its siblings |
| 26 | `glib` | 0.18.5, needs 0.20.0 | ❌ **Still open — blocked upstream** |

### What remains open, and why

**#26 `glib`** — reached via `atk 0.18.2` → `gtk 0.18.2` → `muda` → `tauri`. Even Tauri
2.11.1 still pins the **gtk-rs 0.18** family, and the fix needs gtk-rs 0.20. Nothing to do
until Tauri's Linux stack moves. It is a **Linux/GTK-only** dependency, so Windows and macOS
builds are unaffected. Re-check whenever Tauri is next upgraded.

**#34 may remain partially open**: `rand` 0.7.3 is also in the lock and falls in the affected
range, but it is pulled by `phf_generator` as a **build-time** dependency — not in the runtime
graph and not updatable independently.

### Notes worth keeping

- **The `rand` advisories never threatened password generation.** They require the `log` +
  `thread_rng` features *plus* a custom logger that calls RNG methods on `ThreadRng` during
  reseed, with trace/warn logging on and `getrandom` failing. Flying Carpet defines no custom
  logger. Called out because `rand` mints the single-use transfer passwords, a load-bearing
  security invariant (`docs/shared-network-crypto.md` §7) — that invariant was never
  compromised.
- **#36 (`tauri`) was the only alert with a plausible attack path.** CVE-2026-42184:
  `is_local_url()` misclassified remote URLs as trusted local origins on Windows/Android,
  letting a remote page invoke local-only IPC commands. Exploitability here was low — the app
  loads only local frontend assets (`frontendDist: "../src"`) and never navigates to a remote
  URL — but it is a real fix.
- The Tauri bump is a **62-package delta** (`wry` 0.53.5 → 0.55.1, `tray-icon`, `wasm-bindgen`,
  `web-sys`, `webkit2gtk`, …). Desktop smoke tests should be re-run on all three platforms
  before release.

---

## 3. Side effect: rust-analyzer proc-macro crash (resolved)

Before the toolchain upgrade, VS Code showed `all proc-macro server workers have exited` on
every `#[tauri::command]` and `#[derive(...)]` in `Flying Carpet/src-tauri/src/main.rs`. This
was **not** a code problem — rustc 1.85.0 (Feb 2025) had drifted ~17 months behind the
auto-updating rust-analyzer bundled with the VS Code extension, and the proc-macro bridge ABI
no longer matched, so the server crashed on startup. Upgrading the toolchain fixes it;
restart rust-analyzer afterward ("Developer: Reload Window" or
"rust-analyzer: Restart Server"). If this recurs, suspect toolchain/rust-analyzer version
skew before suspecting the code.
