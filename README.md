## Version 9 adds Bluetooth for transfer negotiation

### Download for Android:

<a href="https://play.google.com/store/apps/details?id=dev.spiegl.flyingcarpet"><img alt="Get it on Google Play" src="screenshots/google-play.png" width="240"/></a>&nbsp;&nbsp;<a href="https://f-droid.org/packages/dev.spiegl.flyingcarpet"><img src="screenshots/f-droid.png" alt="Get it on F-Droid" width="240"></a>

Or if you prefer to sideload, `android_FlyingCarpet_9.0.8.apk` is available on the [releases](https://github.com/spieglt/FlyingCarpet/releases/latest) page.

### Download for iOS:

<a href="https://apps.apple.com/us/app/flying-carpet-file-transfer/id1637377410"><img alt="Get it on Apple App Store" src="screenshots/app-store.png" width="240"/></a>

Or search the App Store for "Flying Carpet File Transfer".

### Linux, macOS, and Windows versions are available on the [releases](https://github.com/spieglt/FlyingCarpet/releases/latest) page. Installers and standalone executable versions available.

# Flying Carpet

Send and receive files between Android, iOS, Linux, macOS, and Windows — either over ad hoc WiFi with a hotspot the devices form themselves, or over a WiFi network both devices already share. No internet or cell connection required, just two devices with WiFi (and optionally Bluetooth) chips in close range.

Don't have a flash drive? Don't have access to a wireless network? Need to move a file larger than 2GB between different filesystems but don't want to set up a network share? Try it out!

[Demo video](https://youtu.be/52Xkrx2BXrg)

## Screenshots:

<img src="screenshots/android.png" width="240"> <img src="screenshots/ios.png" width="240"> <img src="screenshots/linux.png" width="280"> <br> <img src="screenshots/mac.png" width="360"> <img src="screenshots/windows.png" width="360">

## Use:

**Linux:** Download the `.AppImage` file from the [releases](https://github.com/spieglt/FlyingCarpet/releases) page for a standalone version, or if you're on a Debian-based distribution, download the `.deb` file and install it with `dpkg`.

**macOS:** Download the `.dmg` disk image file from the [releases](https://github.com/spieglt/FlyingCarpet/releases) page. Double-click to mount it and drag the `.app` bundle inside to your Applications folder. Or if you use Homebrew, run `brew install flying-carpet`.

**Windows:** Download the `.msi` installer from the [releases](https://github.com/spieglt/FlyingCarpet/releases) page, or `FlyingCarpet.exe` for a standalone version.

## Compilation Instructions:

+ Install [Rust](https://www.rust-lang.org/tools/install).

+ Run `cargo install tauri-cli` to install Tauri.

+ For Linux, install dependencies. Ubuntu 20 example:
```
sudo apt install libsoup2.4* libjavascriptcoregtk* libgdk-pixbuf2.0* librust-pango-sys-dev libgdk3.0* librust-atk-dev librust-atk-sys-dev librust-gdk* libwebkit2gtk* librsvg2-dev
```

+ Run `cargo tauri dev` to run a development version or `cargo tauri build` to create release artifacts.

## Restrictions:

+ Apple devices cannot programmatically run hotspots, so Apple-to-Apple transfers require a shared network: join both devices to the same WiFi network and use shared network mode. That network can itself be a hotspot made manually before running the transfer — for example, start a Personal Hotspot on one phone (or a hotspot on a laptop), join it from the other device, then run Flying Carpet on both.

+ Earlier versions could not use Bluetooth to send from macOS to Linux unless the devices had been manually paired first, with the pairing initiated from the macOS side. This has been fixed[^1].

+ Disables your wireless internet connection while in use. (Does not apply in shared network mode, or to Windows or Android when hosting the hotspot.)

+ macOS sometimes switches back to a wireless network with internet connectivity during particularly long transfers.

+ The Android version requires at least Android 10/API level 29. The Android version does not work on some Xiaomi, MIUI, or HarmonyOS devices, and possibly other Android-like OSes. I don't own these devices and so can't test, but it seems like this is due to lack of support for the [LocalOnlyHotspot](https://developer.android.com/develop/connectivity/wifi/localonlyhotspot) API. It has been confirmed to work on at least one Xiaomi phone.

+ Requires Windows 10 or later.

+ The Linux version was developed and tested on Linux Mint. I mainly intend for it to run on Debian-based distributions. I will try to help troubleshoot others if I can, but I may not be able to as I don't have access to spare machines. There has been at least one [issue](https://github.com/spieglt/FlyingCarpet/issues/64) running on Fedora, possibly related to SELinux but I don't really know.

+ Sometimes when the Cancel button is hit on the desktop platforms, it can take time for the OS to finish trying to join or create a hotspot. Please only click the Cancel button once and wait a few seconds. This sounds like it should be easy to fix, but last time I tried it was not.

## Planned Features

+ Add Flying Carpet shortcut to iOS Share menu.

## Questions That Could Be Asked at Some Point:

+ **Wasn't this a Go repo?** Yes, carcinization has come for the gopher. There were several issues I didn't know how to solve in the Go/Qt paradigm, especially with Windows: not being able to make a single-file executable, needing to Run as Administrator, and having to write the WiFi Direct DLL to a temp folder and link to it at runtime because Go doesn't work with MSVC. Plus it was fun to use `tokio`/`async` and `windows-rs`, with which the Windows networking portions are written. The GUI framework is now Tauri which gives a native experience on all platforms with a very small footprint. The Android version is written in Kotlin and the code is in this repository. The iOS and macOS versions are written in Swift and that codebase is not public.

+ **You're using SHA-256 to derive the key from a password. Isn't that bad? Shouldn't you be using a Password-Based Key Derivation Function like Scrypt or Argon2?** This used to be the case, but transfers are now protected by a [Noise protocol](https://noiseprotocol.org/) `NNpsk0` handshake (X25519, ChaCha20-Poly1305, SHA-256) whose pre-shared key is derived from the transfer password with PBKDF2-HMAC-SHA256 at 600,000 iterations. PBKDF2 rather than Scrypt/Argon2 because the limiting platform is Apple: no third-party crypto is used on iOS/macOS, and CryptoKit has no memory-hard KDF. More importantly, the handshake's ephemeral Diffie-Hellman means a recording of the traffic can't be used to test password guesses offline at all — an attacker has to actively interfere with a live handshake, one guess per attempt — and each transfer gets fresh keys (forward secrecy). SHA-256 of the password is still used, but only to derive the hotspot SSID and to authenticate peer discovery announcements on the local network, not to encrypt anything. See `docs/shared-network-crypto.md` for the full design.

+ **Why encrypt at all if the hotspot is already protected by WPA2?** Because transfers can now also run over a shared network — a café AP, an office LAN — where an in-path attacker is the expected case rather than the exception. The Noise layer protects file contents *and* metadata (file names, sizes, hashes) end to end on any network, and the plaintext preamble that precedes the handshake is bound into it (as the Noise prologue), so tampering with the preamble aborts the transfer. Earlier versions encrypted only file contents with AES-GCM under `SHA256(password)`; that scheme was replaced because it permitted offline password cracking, had no forward secrecy, and left metadata in the clear.

## Complaints at Apple

+ The [documentation](https://developer.apple.com/documentation/corewlan/cwinterface/scanfornetworks(withssid:)) for `scanForNetworks(withSSID:)` does not mention that it requires location permissions.

+ There should be a way to programmatically start hotspots, or at least read the current hotspot configuration with the user's permission.

+ `CBPeripheralManager` advertisements on macOS always use the public Bluetooth address and always declare BR/EDR support, with no API to set the "BR/EDR Not Supported" flag or use a random address. This is what made Linux connect to Macs over classic Bluetooth instead of BLE[^1].

If you've used Flying Carpet, please send feedback to theron@spiegl.dev. Thanks for your interest! Please also check out https://github.com/spieglt/cloaker, https://cloaker.mobi, and https://github.com/spieglt/whatfiles.

[^1]: Root cause: when macOS acts as the Bluetooth LE peripheral (Flying Carpet's sending side), it advertises with its public Bluetooth address and with advertisement flags declaring simultaneous LE and BR/EDR (classic Bluetooth) support — CoreBluetooth provides no way to change either. BlueZ, the Linux Bluetooth stack, counts every such advertisement as a sighting of *both* bearers, and when connecting to an unbonded dual-mode device it breaks the "most recently seen bearer" tie in favor of classic. So Linux always connected to the Mac over classic Bluetooth — which carries none of the app's GATT services — rather than BLE, and the transfer failed to find the Flying Carpet characteristics. (iOS never hit this because it advertises with a random address, which BlueZ always connects over LE; the same was true of Windows and Android peripherals.) Manually pairing from the macOS side only "worked" by leaving behind bond state that happened to steer BlueZ to LE. The fix: when connecting to an unpaired peer that advertises with a public address, the Linux version now creates the bond itself over LE — it opens an LE L2CAP socket requiring high security, which makes the kernel run numeric-comparison pairing (with a confirmation code shown on both devices) on the LE link — and from then on BlueZ prefers the solely-bonded LE bearer.
