# Shared Network Mode: Cryptographic Design

Status: **Phase 1 (Rust core) implemented and verified against the official Noise vectors;
Android and Apple ports pending.** Audience: an engineer implementing this across the Rust
core, the Swift (iOS/macOS) app, and the Kotlin (Android) app.

This document explains *why* the design is shaped the way it is, not just what to build,
so that the person writing the code understands which properties are load-bearing and
which lines they must not "simplify."

---

## 1. Why we're changing anything

Today, shared network mode derives its key as `key = SHA256(password)` and encrypts only
file *contents* with AES-GCM. This was acceptable when the only mode was hotspot, because
the two devices formed their own WPA2 network with no third party in the path. Over a
shared network (a café AP, an office LAN, an operator you don't control) the in-path
attacker is the *expected* case, and the current design fails against it in three ways:

1. **Offline password cracking.** `SHA256(password)` is a single fast hash with no salt
   and no work factor. An eavesdropper who records the TCP stream tries each candidate
   password — `SHA256(guess)` → trial-decrypt one chunk → the AES-GCM tag says
   "right/wrong" instantly. The password space is ~2⁴⁸ (8 chars from a 55-symbol set),
   which a GPU clears in minutes-to-hours.
2. **No forward secrecy.** The key is a pure function of the password, so *every* transfer
   that ever used that password shares one key, and recovering the password (point 1)
   retroactively decrypts all of them.
3. **Cleartext metadata.** Only chunk *payloads* are encrypted. File count, every
   filename and its length, every file size, every chunk length, and the SHA-256 file
   hashes are sent in the clear. A passive observer reads your directory structure and
   file sizes with no cracking at all.

The goal of this design is to fix all three using **only primitives available in Apple's
CryptoKit** (a hard constraint: no third-party crypto on Apple platforms), which rules out
both Argon2id (no memory-hard KDF in CryptoKit) and any PAKE like SPAKE2 (none in
CryptoKit). What we *can* build from CryptoKit — X25519, HKDF, AES-GCM/ChaChaPoly, HMAC —
is a **password-authenticated ephemeral key exchange**. It does not reach full PAKE
security, and §7 is explicit about the one gap that leaves.

---

## 2. Threat model

**Assets we protect:** file contents, filenames, file sizes, file count, hashes — i.e.
everything about what is being transferred.

**Attacker A — passive eavesdropper.** Sees every byte on the wire (same Wi-Fi, span
port, malicious operator) but does not inject or modify traffic. This is the common,
realistic threat on an untrusted network. **We defeat this attacker completely.**

**Attacker B — active in-path attacker.** Can inject, modify, and MITM the TCP connection,
e.g. via ARP spoofing on the LAN, and can impersonate the peer's IP to win the connection.
**We detect this attacker** (they cannot silently sit in the middle and read plaintext),
**but** completing one handshake with a victim hands them an *offline* dictionary attack
on the password (§7). A real PAKE would deny even that; CryptoKit can't give us one.

**Explicitly out of scope:** traffic *volume* and *timing*. The length prefix of each
encrypted record is in the clear (you must know how many bytes to read), so an observer
still sees the approximate total transferred and the record cadence. Hiding that needs
padding/cover traffic, which we do not do.

---

## 3. The building blocks (plain-language)

**X25519 (Elliptic-Curve Diffie–Hellman).** Each side makes a random *ephemeral* keypair:
a secret scalar and a public point (`A = a·G`). They swap public points over the open
channel. Each combines its own secret with the other's public to get the *same* 32-byte
shared secret: `a·B = b·A = ab·G`. The property we rely on: a passive observer who sees
only `A` and `B` **cannot** compute `ab·G` — that's the Computational Diffie–Hellman
assumption, the same one all of TLS rests on. "Ephemeral" = the keypair is generated fresh
per transfer and thrown away after, which is what gives us forward secrecy.

**HKDF (HMAC-based Key Derivation Function).** Turns raw secret material (like the ECDH
output) plus context into one or more uniformly-random keys. Two steps: *extract*
(`salt`, input) → a pseudorandom key, then *expand* (that key, `info` label) → output
key(s). We use the `info` labels for *domain separation*: deriving several independent
keys from one exchange by expanding with different labels. HKDF is **not** a password
hardener — it has no work factor — so it is never used *alone* on the password.

**AEAD (AES-256-GCM or ChaCha20-Poly1305).** Symmetric encryption that provides
confidentiality *and* integrity: each encryption produces ciphertext plus a 16-byte
authentication tag; decryption fails loudly if any bit was altered. Each encryption takes
a **nonce** that must **never repeat under the same key** — nonce reuse in GCM is
catastrophic (it leaks the XOR of plaintexts and can forge tags). Nonce management is the
single most dangerous part of this design to get wrong; see §6.

**HMAC.** A keyed integrity tag. We use it for *key confirmation*: proving each side
derived the same session key (which simultaneously proves each side knew the password).

All five are in CryptoKit (`Curve25519.KeyAgreement`, `HKDF`, `AES.GCM` / `ChaChaPoly`,
`HMAC`) and in RustCrypto (`x25519-dalek`, `hkdf`, `aes-gcm`/`chacha20poly1305`, `hmac`).
On Android they come from JCA (`KeyAgreement "XDH"` for X25519, API 30+; `Mac "HmacSHA256"`;
`Cipher "AES/GCM/NoPadding"`) plus BouncyCastle for HKDF (or hand-rolled from HMAC) —
**not** Google Tink: Tink deliberately hides raw ECDH (its Java `X25519` lives in the
`subtle`/`@Alpha` namespace marked "do not use in production, may be removed at any time,"
and Tink Java has no production X25519 key-agreement key type). Tink is a high-level
misuse-resistant library (AEAD, HPKE, MAC, signatures) and is the wrong abstraction level
for hand-assembling a handshake. In practice you would not source these individually on
Android at all — see §8.

---

## 4. The handshake, step by step

Roles are already fixed by the existing protocol: the **receiver is the TCP server**, the
**sender is the TCP client**. They already share the password (receiver displays it,
sender types it) and already have a TCP connection from discovery. The handshake is the
first thing that runs on that connection, before any file metadata is exchanged.

```
Sender (TCP client)                         Receiver (TCP server)
        |                                            |
   gen ephemeral (a, A=a·G)                     gen ephemeral (b, B=b·G)
        |                                            |
        |---------------- A (32 bytes) ------------->|
        |<--------------- B (32 bytes) --------------|
        |                                            |
   dh = X25519(a, B)                            dh = X25519(b, A)
        |          (both now hold ab·G)              |
        |                                            |
   derive keys = HKDF(  ikm = dh || pw,          (identical derivation)
                        salt = transcript,
                        labels... )
        |                                            |
        |------ confirm_sender = HMAC(kc_s, T) ----->|  verify; abort if bad
        |<----- confirm_recv   = HMAC(kc_r, T) ------|
   verify; abort if bad                              |
        |                                            |
        |=========== encrypted record layer =========|
        |   all file metadata + all file data,        |
        |   AES-GCM under directional keys            |
```

Where:

- `pw` is the password material (see §5 on whether to PBKDF2 it first).
- `transcript = A || B` (the two 32-byte public keys in a fixed order — client's then
  server's). Binding the transcript into the derivation ties the derived keys to *this
  specific exchange*, which is what stops an attacker from replaying or reflecting one
  side's messages.
- `T` is a confirmation transcript, e.g. `"FC-v10-confirm" || A || B`.

**Why the cleartext public-key swap is safe.** `A` and `B` go over the wire before any key
exists, so an active attacker (B) can swap in their own public key — the classic
unauthenticated-DH MITM. That is exactly what the **password-bound key confirmation**
catches: if the attacker substitutes their key, the victim derives its session key from
`(attacker's DH) || password`, and the attacker would need the *password* to produce a
matching `HMAC` confirmation. They don't have it, so confirmation fails and the transfer
aborts. The password is what turns an otherwise-unauthenticated DH into an authenticated
one.

---

## 5. Key derivation — exact recipe

Do **not** invent the mixing yourself beyond this recipe. Better still, see §8: model the
whole handshake on the Noise Protocol Framework's PSK pattern, which specifies all of the
below in a reviewed way and is buildable from these same primitives.

```
ikm  = dh (32 bytes)  ||  pw_material          # dh MUST be included -> passive immunity
salt = transcript      = A || B                 # 64 bytes, same on both sides
prk  = HKDF-Extract(salt, ikm)

# Four independent keys, one HKDF-Expand each, distinct info labels:
k_s2c    = HKDF-Expand(prk, "FC-v10 sender->receiver data",     32)
k_r2s    = HKDF-Expand(prk, "FC-v10 receiver->sender data",     32)
kc_sender  = HKDF-Expand(prk, "FC-v10 sender confirm",   32)
kc_recv    = HKDF-Expand(prk, "FC-v10 receiver confirm", 32)
```

- **Hash:** SHA-256 throughout (HKDF-SHA256, HMAC-SHA256).
- **AEAD:** pick **one** and hard-code it for interop. AES-256-GCM is the safe default
  (hardware-accelerated everywhere, in CryptoKit as `AES.GCM`). ChaCha20-Poly1305 is an
  equally fine alternative; do not make it negotiable in v1.
- **`pw_material`:** at minimum the raw UTF-8 password bytes. **Recommended:**
  `pw_material = PBKDF2-HMAC-SHA256(password, salt = transcript, iterations = 600_000)`.
  PBKDF2 is available on Apple via CommonCrypto (a system library) and in RustCrypto / JCA.
  Note carefully *what this does and does not buy* (§7): it only slows the **active**
  attacker's offline crack; the **passive** attacker already has no oracle regardless of
  KDF, because they can't compute `dh`. So PBKDF2 here is defense-in-depth for the one
  residual case, not the primary defense. Using the transcript as the PBKDF2 salt is fine —
  a salt need not be secret, only unique per session, which the ephemeral transcript
  guarantees (and it prevents any precomputation).

**Key confirmation.** Sender sends `HMAC(kc_sender, "FC-v10 confirm" || A || B)`; receiver
recomputes and compares in constant time, aborting on mismatch, then sends
`HMAC(kc_recv, ...)` which the sender likewise verifies. A mismatch means wrong password
**or** an active MITM — the user-facing message should say "could not establish a secure
connection; check the password and that you trust this network," not "wrong password"
(the two are indistinguishable here, and that's fine).

---

## 6. The record layer (this is where metadata gets encrypted)

The important architectural point: **we do not rewrite the send/receive protocol.** The
existing logic — send file count, send filename length, send filename, send size, send
chunks — runs unchanged, but writes into an *encrypting wrapper* instead of the raw
socket. Every logical message becomes one AEAD record:

```
on the wire:  [ 2-byte big-endian length ] [ ciphertext || 16-byte AEAD tag ]
```

> Implementation note: the shipped Rust reference builds this record layer from Noise's
> transport messages (§8), so the "directional keys" and "counter nonces" below are
> managed by Noise/`snow` internally rather than derived by hand, and the length prefix is
> **2 bytes** (`u16`) because Noise caps a message at 65535 bytes. §4–§6 are the conceptual
> model; §9 lists the exact framing and test vectors the implementation actually uses.

- **Directional keys.** Sender→receiver records use `k_s2c`; receiver→sender records use
  `k_r2s`. Two keys means the two nonce counters live in separate spaces and can never
  collide — this is the clean way to avoid nonce reuse between the two directions.
- **Counter nonces, not transmitted.** Each direction keeps a 96-bit counter starting at
  0, incremented by exactly one per record sent. The nonce is the counter, big-endian; it
  is **not** put on the wire (both sides track it deterministically, like TLS 1.3 sequence
  numbers). Never reset a counter mid-connection. Never reuse a key across connections
  (the ephemeral handshake guarantees fresh keys each time).
- **GCM data limit.** Stay well under ~2³⁹ bytes (~64 GB) encrypted under a single
  directional key before the GCM security margin degrades. Flying Carpet transfers are far
  below this; if you ever expect larger, that's the point to add rekeying, but do not add
  it speculatively.
- The plaintext inside each record is exactly what the current protocol writes today:
  the u64 file count, the u64 filename length + filename bytes, the u64 file size, and the
  file chunks. All of it is now confidential and tamper-evident. The header-value bounds
  we already added (file count / filename length / chunk size sanity checks) still apply
  to the *decrypted* values.

---

## 7. What this achieves — and why the residual gap barely matters here

| Property | Status |
|---|---|
| Confidentiality of file **contents** | ✅ AEAD under session key |
| Confidentiality of **metadata** (names, sizes, count, hashes) | ✅ record layer |
| Integrity / tamper-evidence | ✅ AEAD tags + key confirmation |
| Forward secrecy (past transfers safe if password later leaks) | ✅ ephemeral X25519 |
| Offline password crack by **passive** eavesdropper (Attacker A) | ✅ **impossible** |
| Active MITM goes undetected | ✅ **prevented** (key confirmation) |
| Offline password crack by **active** attacker (Attacker B) | ⚠️ possible, but **yields nothing of value** — see below |

**The abstract residual gap.** An active attacker (Attacker B) who *terminates* one side's
connection and runs the DH with the victim themselves receives that victim's
key-confirmation HMAC, keyed under a DH secret the attacker knows. They can then test
passwords *offline* against it. This is inherent to any password-authenticated exchange
that is **not** a PAKE: someone must send their confirmation first, and the recipient of
that first message gets an offline oracle. In a system with **long-term or reused**
passwords this would matter — crack once, impersonate forever — and a formally-proven PAKE
(SPAKE2, CPace) is what removes it, by limiting an active attacker to one *online* guess
per connection. A PAKE is precisely what the CryptoKit-only constraint excludes.

**Why it has no operational payoff in Flying Carpet.** Two properties of this specific
design neutralize the gap:

1. **The password is single-use and randomly generated.** The receiver mints a fresh
   CSPRNG password per transfer and displays it out-of-band; it is never reused. The
   offline crack is, by construction, not real-time — PBKDF2 makes each guess cost ~600k
   hashes, so recovering the password takes far longer than the transfer it belonged to.
   By the time the attacker holds it, it authenticates nothing, decrypts nothing, and
   predicts nothing (CSPRNG output doesn't leak future outputs). The "crack once,
   impersonate later" payoff requires the reuse this design doesn't have.

2. **Oracle and ciphertext are mutually exclusive for a given transfer.** To obtain the
   offline oracle, the attacker must **terminate** the victim's connection and do their own
   DH — which means the transfer aborts at key confirmation, *before any file data or even
   any metadata record is sent.* They get one MAC and nothing else — no file, no filenames,
   no sizes. To obtain the actual ciphertext of a *completed* transfer, the attacker must
   instead **transparently relay** the two real endpoints' messages — but then the session
   key comes from the genuine endpoint-to-endpoint DH secret, which they cannot compute
   (the same CDH wall the passive attacker hits), so there is no crackable oracle. Getting
   the oracle costs them the data; getting the data costs them the oracle. For
   confidentiality, the active attacker is no better off than the passive one.

**Net.** Against every attacker this design actually faces — passive eavesdropper, and
active LAN attacker who intercepts a live transfer — cracking the recovered password yields
nothing of value. The construction is therefore, *for Flying Carpet's single-use-password
model*, effectively as strong as a PAKE would be here. A PAKE would add cleaner theory and
defense-in-depth, but no practical protection against any realistic attacker — which
substantially weakens the case for taking on the SPAKE2 cross-language burden.

**Load-bearing invariants.** If either is ever broken, the reasoning above collapses and
the case for a PAKE returns:

- **Passwords must remain single-use.** No "remember this password," no user-chosen fixed
  passwords, no reuse across transfers. Treat this as a security invariant of the mode, not
  a UI convenience — the entire "the crack is worthless" argument rests on it.
- **Passwords must come from a CSPRNG** (`rand::thread_rng` / `SecureRandom` /
  `SecRandomCopyBytes`), so recovering past passwords never helps predict future ones.

**Caveats that remain regardless.** An active attacker can still *disrupt* a transfer —
intercept-and-abort is a denial of service available to anyone in-path, unrelated to the
crypto and not fixable by it. And this is still a hand-assembled handshake, not a proven
PAKE; §8 is why it should be built on the Noise pattern rather than free-handed.

---

## 8. Strong recommendation: build it as a Noise pattern, don't free-hand it

The Noise Protocol Framework specifies handshakes that are exactly "ephemeral X25519 + HKDF
+ AEAD, with an optional pre-shared key mixed in," and it pins down the transcript hashing,
the key schedule, the nonce discipline, and the directional keys in a reviewed, widely-
implemented way. The `NNpsk0` pattern (both sides ephemeral-only, a PSK folded in at the
start) is essentially §4–§6 of this document, formalized. Feed `pw_material` in as the
Noise PSK.

Why this matters: Noise's security profile with a low-entropy PSK is **the same** as our
hand-rolled construction — passive-safe, active-attacker offline-crackable (a PSK-Noise
pattern is not a PAKE) — so we lose nothing on security, but we gain a specified,
peer-reviewed recipe for the mechanics that are easy to get subtly wrong (transcript
binding, nonce counters, key separation).

### Per-platform implementation plan

Because Noise is a *spec*, conforming implementations of the same protocol name interoperate.
Fix the protocol name up front — proposed: **`Noise_NNpsk0_25519_ChaChaPoly_SHA256`**
(ChaChaPoly because it is Noise's most universally-tested cipher and is in CryptoKit as
`ChaChaPoly`; AES-GCM is an equally valid pin if preferred). Then:

- **Rust core → use `snow`.** Mature, ~1.4M downloads, tracks Noise spec rev 34, actively
  maintained (0.10.0 released mid-2026), supports the `NNpsk0` pattern. Pure-Rust crypto by
  default, no C deps. Caveat: no formal audit (but it is the de-facto Rust Noise library,
  widely deployed). This is the reference implementation the other two match against.
- **Android → use a Noise library, not raw Tink.** `noise-java` (rweather) is the plain-Java
  reference implementation; it uses JCE where available and ships pure-Java fallbacks for
  primitives an older Android JDK lacks (notably Curve25519), so it works across Android
  versions without an API-30 floor or a BouncyCastle dependency. `java-noise` (jchambers) is
  a newer alternative. Either gives you the whole handshake; do **not** try to assemble it
  from Tink (§3).
- **Apple → hand-implement `NNpsk0` on CryptoKit.** The "Apple standard crypto only"
  constraint forbids a third-party Noise library, and there is no well-established CryptoKit
  Noise library anyway. But Noise is small: the handshake needs only X25519
  (`Curve25519.KeyAgreement`), HKDF (`HKDF`), the AEAD (`ChaChaPoly`), and SHA-256/HMAC —
  all present. You implement the Noise *symmetric state* (the `MixHash`/`MixKey`/`Split`
  key schedule and the PSK token) by hand, following the spec. This is the one hand-rolled
  side and the main source of interop risk, which is exactly what the §9 test vectors guard.

The PSK fed into `NNpsk0` is 32 bytes: `psk = HKDF-or-PBKDF2(password) → 32 bytes`. Use
PBKDF2 (§5) so the active-attacker offline crack is slowed; derive it identically on all
three sides (same iteration count, same salt). Everything else — nonce counters, directional
keys, transcript binding — is handled *by the Noise pattern itself*, which is the whole
reason to use it rather than the §4–§6 hand-rolled version. Treat §4–§6 as the conceptual
model; treat the Noise `NNpsk0` spec as the normative reference for the bytes.

---

## 9. Interop and testing (the part that actually eats the time)

Because the handshake is standard Noise (§8), most byte-level details are fixed by the
Noise spec and the protocol name. What each platform must still pin identically:

- **Protocol name:** `Noise_NNpsk0_25519_ChaChaPoly_SHA256` (exact string).
- **Prologue:** **empty** (no `.prologue()` call). All platforms must use an empty
  prologue, or the handshake hashes diverge and nothing interoperates.
- **PSK derivation:** `PBKDF2-HMAC-SHA256(password_utf8, salt, iters) → 32 bytes`, with
  `salt = b"Flying Carpet v10 shared network PSK"` and `iters = 600000`.
- **Record framing:** each Noise message is prefixed by its length as a **2-byte
  big-endian** integer (`u16`), tag appended (not prepended). 2 bytes, not 4, because
  Noise caps messages at 65535 bytes; the same framing is used for the two handshake
  messages and every transport record.
- **Roles:** the shared-network sender (TCP client) is the Noise **initiator**; the
  receiver (TCP server) is the **responder**.

**Spec conformance is verified against the official vectors.** `core/src/noise.rs` test
`official_noise_test_vector` drives `snow` with the canonical
`Noise_NNpsk0_25519_ChaChaPoly_SHA256` test vector from
[haskell-cryptography/cacophony](https://github.com/haskell-cryptography/cacophony)
(`vectors/cacophony.txt`) — its prologue, PSK, and fixed ephemerals — and
asserts the handshake message ciphertexts, the handshake hash
(`f4d03dc3…be208eaf`), and the first transport record match byte-for-byte. This is what
guarantees the Swift/Kotlin ports (which follow the same spec) interoperate; **each port
should run the same cacophony vector against its own Noise implementation.** The
app-specific KAT vectors below (empty prologue, PBKDF2-derived PSK) are a *separate*
determinism check for our exact parameters, not a spec check.

**Cross-platform known-answer test vectors.** These are emitted and asserted by the Rust
reference (`core/src/noise.rs`, tests `psk_known_answer` and `handshake_known_answer`);
Swift and Kotlin must reproduce them exactly. `snow`'s
`fixed_ephemeral_key_for_testing_only` fixes the ephemerals so the whole handshake is
deterministic.

- PSK for password `"flyingcarpet"`:
  `a3d8b7f17f2252e4c2847a365ab2f392beaa996b7e51dd6fa19ff1ad08938619`

With fixed PSK = `2a`×32, initiator ephemeral private = `01`×32, responder ephemeral
private = `02`×32:

- Handshake msg 1 (initiator → responder), 48 bytes:
  `a4e09292b651c278b9772c569f5fa9bb13d906b46ab68c9df9dc2b4409f8a209a3e9c18456aba2185de800ffaca55b22`
- Handshake msg 2 (responder → initiator), 48 bytes:
  `ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59d887595caf8a0b110dfab84e6b41eafc`
- First transport record from the initiator, plaintext `"hello flying carpet"`:
  `124a00c03b4544f746828bbf9ae2d8d595a9ac1fea988f43f7206c3880180b954f9147`

(The transport-record vector is the raw Noise message; on the wire it is preceded by its
2-byte length prefix `0023`.) This is the vector that guarantees a macOS sender can talk to
an Android receiver; port it into an XCTest / `DiscoveryUnitTest`-style test on each
platform before wiring live transfers, since it catches label and endianness mismatches
that otherwise surface as "handshake fails, no idea why."

---

## 10. Migration, scope, and locked decisions

**Decisions locked** (from design review):

- **Cipher: ChaCha20-Poly1305.** Noise protocol name **`Noise_NNpsk0_25519_ChaChaPoly_SHA256`**.
  (AES-GCM is also in CryptoKit and would be equally valid; ChaChaPoly chosen as Noise's
  most-tested cipher.)
- **No v9 compatibility.** v10 is a clean break; a v9 peer is rejected with a clear message.
- **Discovery unchanged for now.** Still `HMAC(SHA256(password), announcement)`; it reveals
  only presence/IP, never file data. Folding it into the hardened scheme is possible later,
  out of scope here.
- **Hotspot mode unaffected.** WPA2 around the link; nothing here touches
  `ConnectionMode::Hotspot`.

**Version bump — already implemented** (the only code landed ahead of the Noise work):

- `MAJOR_VERSION` 9 → 10 in the Rust core (`core/src/lib.rs`), Android (`MainViewModel.kt`),
  and Apple (`Transfer.swift`, `VERSION`).
- Compatibility floor raised to 10 (`utils::is_compatible`, the Kotlin `peerVersion >= 10`
  check, and Swift `isCompatible`), so a v9 peer is incompatible.
- Version mismatch now produces a clear, user-facing message naming both versions and the
  download page, on all three platforms.

**A versioning subtlety to respect.** The version number only protects the Noise break if
the Noise wire format never ships under the *same* number as a non-Noise build:

- The Noise change must land as part of **v10** — do not *release* a v10 build before Noise
  is in. If a v10 is ever released pre-Noise, the Noise wire change must bump to **v11**,
  because two builds both reporting "10" with different record-layer formats would not
  detect the mismatch; they'd fail cryptically instead.
- There is no interop between v10's encrypted handshake and v9's cleartext protocol, and
  hotspot-vs-shared-network is chosen locally before any bytes flow, so v10 just runs its
  protocol and a v9 peer fails the version exchange with the message above. No downgrade
  path.

**The logical send/receive protocol does not change** — only the transport under it. The
header-value bounds and filename sanitization already added remain, applied to decrypted
values.

---

## 11. Implementation plan (phased)

Order chosen so the cheapest, most-verifiable piece comes first and becomes the reference
the others are tested against.

### Phase 0 — done
Version bump + mismatch messaging (§10). The only pre-Noise code change.

### Phase 1 — Rust core reference implementation (`snow`) — **done**
Landed in `core/src/noise.rs` and wired into `core/src/lib.rs`:
1. `snow` and `pbkdf2` added to `core/Cargo.toml`; protocol
   `Noise_NNpsk0_25519_ChaChaPoly_SHA256`.
2. `derive_psk()` = PBKDF2-HMAC-SHA256 with the fixed salt/iters constants (§9).
3. The handshake runs in `start_transfer`'s shared-network arm, right after
   `start_shared_network_transfer` returns the TCP stream and *before* `confirm_version`.
   Sender = initiator, receiver = responder. A wrong password fails the handshake with a
   clear message ("Could not establish a secure connection. Check that the password
   matches…").
4. `EncryptedStream<S>` implements tokio `AsyncRead + AsyncWrite`, transparently splitting
   the byte stream into ≤64 KiB Noise records with a 2-byte length prefix. `send_file` /
   `receive_file` / `confirm_version` / `confirm_mode` are now generic over the stream, so
   their bodies are unchanged; a `TransferStream` enum (`Plain` for hotspot, `Encrypted`
   for shared network) keeps a single stream binding. Hotspot mode is byte-for-byte
   unchanged.
5. **Spec conformance verified against the official Noise vectors:**
   `official_noise_test_vector` drives `snow` with the canonical cacophony vector for the
   protocol and asserts the message ciphertexts, handshake hash, and transport record
   match byte-for-byte — this is the real cross-platform interop guarantee. Additional
   app-parameter KAT vectors (`psk_known_answer`, `handshake_known_answer`) are transcribed
   into §9 as a determinism check.
6. Verified: `end_to_end_encrypted_transfer` runs the real send/receive over an encrypted
   duplex with a 200 KB (multi-record) file; `wrong_password_fails_handshake`,
   `round_trip_small_and_large`, and `tampering_is_detected` all pass. 18 core tests green.

**Known Phase-1 redundancy (intentional, revisit later):** the file *chunks* are now
double-encrypted — the unchanged `send_file`/`receive_file` still AES-256-GCM each chunk
with `SHA256(password)`, and the Noise layer then ChaChaPoly-wraps everything. This is the
cost of "don't rewrite the logical protocol"; the metadata protection (the actual goal) is
achieved. A later cleanup could drop the inner AES-GCM once all platforms are on Noise, but
that is itself a wire change and out of scope for getting parity.

### Phase 2 — Android (`noise-java`)
1. Add `noise-java` (or `java-noise`) to Gradle; same protocol name.
2. Same PSK derivation (identical salt/iters); verify against the Phase 1 vector in a
   `DiscoveryUnitTest`-style unit test *before* wiring anything live.
3. Wrap the socket streams (`Receive.kt` / `Send.kt` / `MainViewModel.startTransfer`) in a
   Noise transport mirroring the Rust `EncryptedStream`. Sender = initiator, receiver =
   responder.
4. Verify Android↔Windows both directions against the Phase 1 reference.

### Phase 3 — Apple (hand-rolled on CryptoKit)
1. Implement the `NNpsk0` handshake + transport from `Curve25519.KeyAgreement`,
   `HKDF<SHA256>`, `ChaChaPoly`, and `HMAC<SHA256>`, following the Noise spec's
   `MixHash`/`MixKey`/`Split` symmetric-state schedule and the `psk` token. No third-party
   library (Apple-standard-crypto constraint).
2. Port the Phase 1 test vector into an XCTest and make it pass **before** any live transfer
   — this is the highest-risk step and the vector is the guard.
3. Same PSK derivation via CommonCrypto PBKDF2 (identical salt/iters).
4. Wrap the `NWConnection` I/O (`TCPConnectionWrapper` / `TCPServer`) in the Noise transport.
5. Verify macOS↔Android and iOS↔Windows both directions.

### Phase 4 — cross-matrix + cleanup
Full matrix (Win / Linux / macOS / iOS / Android × sender / receiver), wrong-password and
version-mismatch paths, and a per-file size larger than one record. Remove the old cleartext
shared-network transport.

### Open items — resolved
- **PBKDF2 salt** → **fixed domain string** `b"Flying Carpet v10 shared network PSK"`. The
  Noise handshake hash already binds the ephemeral transcript, so the salt's only job is
  domain separation and a fixed value is simplest to keep byte-identical across languages.
- **`snow` audit posture** → **accepted.** `snow` has no formal audit (it says so), but it
  is the de-facto Rust Noise library and widely deployed; the fallback if that ever becomes
  unacceptable is a hand-rolled Rust implementation matched to the Apple one.

---

## 12. Summary for the implementer

1. Build the handshake as Noise **`Noise_NNpsk0_25519_ChaChaPoly_SHA256`** (§8), not the
   §4–§6 hand-rolled mixing — treat §4–§6 as the conceptual model and the Noise spec as the
   normative reference for the bytes.
2. The PSK is `PBKDF2(password)` → 32 bytes (§5), derived identically on all three platforms.
3. Wrap the socket so the existing send/receive logic runs unchanged over the Noise
   transport (§6); the receiver is the responder, the sender the initiator.
4. Write the cross-language known-answer vector **first** (§9); it is what makes a macOS
   sender talk to an Android receiver, and the guard for the hand-rolled Apple side.
5. Follow the phased order (§11): Rust `snow` reference → Android `noise-java` → Apple
   CryptoKit hand-roll → full matrix.
6. Version is already bumped to a clean-break v10 with mismatch messaging (§10); ship Noise
   *within* v10 (or bump to v11 if v10 releases first).
7. Know exactly what you're shipping (§7): passive eavesdroppers fully defeated; the active
   attacker's offline oracle is neutralized by single-use passwords, so this is effectively
   as strong as a PAKE would be *for this design* — provided passwords stay single-use and
   CSPRNG-generated.
