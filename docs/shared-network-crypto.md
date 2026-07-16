# Shared Network Mode: Cryptographic Design

Status: **Implemented on all three platforms** (Rust core, Android/Kotlin, Apple/Swift),
each verified byte-for-byte against the official Noise (cacophony) vector and shared app
KATs, including the preamble→prologue binding and tamper-negative tests (§11 post-review
hardening). Windows↔Android confirmed on real hardware pre-binding; first post-binding
live transfer re-confirms. Remaining: the rest of the live cross-platform matrix (§11
Phase 4), and porting the PSK-derived discovery HMAC key (§7, §9) to the Apple repo —
Rust and Android already have it. Audience: an engineer working across the Rust core, the
Swift (iOS/macOS) app, and the Kotlin (Android) app.

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
realistic threat on an untrusted network. **We defeat this attacker:** they read nothing,
and their only recourse is a PBKDF2-hardened offline password crack (§7) that cannot
plausibly finish within the single-use password's lifetime — and that, thanks to forward
secrecy, decrypts nothing even if it someday does.

**Attacker B — active in-path attacker.** Can inject, modify, and MITM the TCP connection,
e.g. via ARP spoofing on the LAN, and can impersonate the peer's IP to win the connection.
**We detect this attacker** (they cannot silently sit in the middle and read plaintext),
**but** completing one handshake with a victim hands them an *offline* dictionary attack
on the password (§7). A real PAKE would deny even that; CryptoKit can't give us one.

**Explicitly out of scope:** traffic *volume* and *timing*. The length prefix of each
encrypted record is in the clear (you must know how many bytes to read), so an observer
still sees the approximate total transferred and the record cadence. Hiding that needs
padding/cover traffic, which we do not do. The **plaintext preamble** (protocol version
and send/receive direction) that precedes the handshake is visible to an eavesdropper —
these values are not secret — but it is not tamperable in any useful way: every preamble
byte is bound into the Noise **prologue**, so modifying it in flight fails the handshake
(§7).

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
  Note carefully *what this buys*: in **this §4–§6 construction**, the passive attacker
  has no oracle regardless of KDF (the first password-dependent value on the wire is keyed
  under `dh`, which they can't compute), so PBKDF2 only slows the **active** attacker's
  offline crack. **The implemented `NNpsk0` design does not share that property** — its
  first handshake message is checkable by a passive observer (§7), which makes PBKDF2 the
  primary defense there, not defense-in-depth. Using the transcript as the PBKDF2 salt is fine —
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

> **Wire-order note (implemented design).** The version and send/receive mode are
> negotiated in a **plaintext preamble** on the raw TCP socket *before* the Noise
> handshake; everything after the handshake — file count, all metadata, and all file data
> — is inside Noise. This holds for **both** hotspot and shared network mode: they are
> identical past the preamble. Every preamble byte, sent and received, is recorded and
> bound into the Noise **prologue** (§9), so the preamble is readable in the clear but not
> silently modifiable: tampering makes the two sides' prologues differ, which fails the
> handshake. See §10 for why the preamble is plaintext and §7 for the security discussion.

The important architectural point: **we do not rewrite the send/receive protocol.** The
existing logic — send file count, send filename length, send filename, send size, send
chunks — runs unchanged, but writes into an *encrypting wrapper* instead of the raw
socket. The chunks are sent as **raw bytes** (no application-level cipher); Noise is the
sole encryption layer. Every logical message becomes one AEAD record:

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
| Offline password crack by **passive** eavesdropper (Attacker A), *from the Noise channel* | ⚠️ **PBKDF2-hardened** — the first handshake message's AEAD tag is an offline oracle (see below) |
| Offline password crack by **passive** eavesdropper, *from the discovery announcement* | ⚠️ **PBKDF2-hardened** (same 600k-iteration cost) — see note below |
| Active MITM goes undetected | ✅ **prevented** (key confirmation) |
| Offline password crack by **active** attacker (Attacker B) | ⚠️ possible at the same PBKDF2 cost, and **yields nothing of value** — see below |

**Discovery is keyed from the stretched PSK (shared network only).** The Noise channel
rows above are not the whole wire: the UDP discovery announcement is also HMAC-signed with
a password-derived key. It used to be `HMAC(SHA256(password), …)` — a *fast* hash, which
gave a **passive** eavesdropper who captured a single announcement an offline dictionary
attack cheap enough (~hours on one GPU, minutes on a rig, over the ~2⁴⁸ space) to
plausibly finish **while the password was still live** (receiver waiting, or mid-transfer
of something large). A live recovered password defeats everything: the attacker knows the
PSK and can run a fully valid MITM that passes key confirmation. This was the one scenario
that broke the "effectively as strong as a PAKE" argument below. Fixed: the discovery HMAC
key is now `derive_discovery_key(psk) = HMAC-SHA256(psk, "Flying Carpet v10 discovery")`
(§9), where `psk` is the PBKDF2-stretched key — so a captured announcement costs an
offline attacker 600k PBKDF2 iterations per guess, identical to the handshake-message
oracle below: centuries per GPU, not hours. The label gives domain separation (the
Noise PSK itself is never used outside the handshake). No fast hash of the password goes
on the air; `SHA256(password)` survives only in the hotspot SSID's 2-byte tag, which is
not part of shared network mode.

*Why not drop discovery authentication entirely?* Considered and rejected. It would not
remove the passive oracle — `NNpsk0` message 1 carries the same PSK-keyed tag in every
recorded transfer regardless (below); it would only shrink the *live* oracle window from
minutes (announcements start once the receiver has the password) to milliseconds (message
1 immediately precedes handshake completion). Nor does the HMAC gate online guessing: the
receiver's TCP port accepts direct connections from anyone, and Noise limits any connector
to one online guess either way. What the HMAC actually buys is **peer selection**:
the sender connects only to the machine that provably holds the password, so concurrent
transfers on one LAN can't cross-connect and mutually fail, and an in-LAN mischief-maker
can't answer discovery first to make every transfer die at the handshake. That reliability
is worth a PBKDF2-hardened oracle window measured in minutes against a crack measured in
GPU-years.

**The residual gap: the wire carries PBKDF2-hardened password oracles.** The §4–§6
conceptual design had the property that a *passive* observer gets no oracle at all — the
first password-dependent value there is keyed under `dh`, which they can't compute. The
implemented `NNpsk0` pattern does **not** have that property. Per the Noise spec (§9.1),
in `psk` handshake patterns the `e` token additionally calls `MixKey(e.public_key)`, so
the initiator's first message — 32-byte ephemeral plus a 16-byte AEAD tag over an empty
payload — is keyed by a function of (protocol name, prologue, PSK, e.pub): everything
public except the PSK. A passive eavesdropper who records message 1 can test passwords
offline: PBKDF2(guess) → run the key schedule → check the tag. The discovery announcement
gives the same oracle even earlier (above). And an active attacker (Attacker B) who
*terminates* one side's connection gets the equivalent oracle from the victim's handshake
message — this last one is inherent to any password-authenticated exchange that is **not**
a PAKE: someone must send a password-dependent message first, and its recipient gets an
offline oracle. Every one of these oracles costs the same — one 600k-iteration PBKDF2 per
guess, ~centuries of GPU time over the ~2⁴⁷ space. In a system with **long-term or
reused** passwords the oracle would still matter — crack once (however slowly),
impersonate forever — and a formally-proven PAKE (SPAKE2, CPace) is what removes it, by
limiting even an active attacker to one *online* guess per connection with nothing
crackable ever hitting the wire. A PAKE is precisely what the CryptoKit-only constraint
excludes.

**Why it has no operational payoff in Flying Carpet.** Two properties of this specific
design neutralize the gap:

1. **The password is single-use and randomly generated.** The receiver mints a fresh
   CSPRNG password per transfer and displays it out-of-band; it is never reused. The
   offline crack is, by construction, not real-time — PBKDF2 makes each guess cost ~600k
   hashes, so recovering the password takes far longer than the transfer it belonged to.
   By the time the attacker holds it, it authenticates nothing, decrypts nothing, and
   predicts nothing (CSPRNG output doesn't leak future outputs). The "crack once,
   impersonate later" payoff requires the reuse this design doesn't have.

2. **A cracked password never decrypts recorded traffic.** A recorded session *does*
   contain the oracle — message 1's tag rides in every taped transfer — so an attacker
   can, in principle, spend the GPU-centuries and recover the password of a transfer they
   recorded. It buys nothing retroactive: the transport keys also depend on the ephemeral
   `ee` DH secret, which no amount of password knowledge reveals (the same CDH wall the
   passive attacker hits — this is forward secrecy doing its job). A recovered password is
   only useful *prospectively*: impersonating an endpoint of, or MITMing, a handshake that
   hasn't happened yet. So the crack would have to finish inside the password's live
   window — from first discovery broadcast to handshake completion, seconds to minutes —
   against a cost of years-to-centuries of GPU time. Outside that window, the single-use
   password authenticates nothing, decrypts nothing, and predicts nothing.

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
crypto and not fixable by it. The **plaintext preamble** (§6) is *bound into the Noise
prologue* (§9): an attacker who flips a bit in the version or mode exchange makes the two
sides compute different prologues, and the handshake fails — tampering is detected, never
silently accepted. For v10↔v10 alone this only converts one DoS into another (the preamble
carries no secret, and each side decided its own send/receive role locally; the exchange
only *verifies* they're opposite), but the binding is load-bearing for the future: when a
later version changes anything crypto-relevant, an in-path attacker must not be able to
rewrite the version exchange and silently pin both peers to v10 semantics. Prologue
binding only closes that downgrade window if it exists from the *first* Noise version —
which is why it ships in v10 rather than being retrofitted.

---

## 8. Strong recommendation: build it as a Noise pattern, don't free-hand it

The Noise Protocol Framework specifies handshakes that are exactly "ephemeral X25519 + HKDF
+ AEAD, with an optional pre-shared key mixed in," and it pins down the transcript hashing,
the key schedule, the nonce discipline, and the directional keys in a reviewed, widely-
implemented way. The `NNpsk0` pattern (both sides ephemeral-only, a PSK folded in at the
start) is essentially §4–§6 of this document, formalized. Feed `pw_material` in as the
Noise PSK.

Why this matters: Noise's security profile with a low-entropy PSK is **nearly the same** as
our hand-rolled construction — not a PAKE, so the password is offline-crackable at PBKDF2
cost. The one difference: `NNpsk0` exposes that oracle to a *passive* observer via the
first message's AEAD tag, where the §4–§6 construction keyed everything checkable under
`dh` (§7) — a difference with no operational impact given the crack economics. In exchange
we gain a specified,
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
- **Android → hand-rolled on JCA + a vendored X25519** (see §11 Phase 2 for the full
  rationale). The obvious libraries don't fit `minSdk 29` / Java 8 / modern `NNpsk0`:
  `noise-java` (rweather) implements the *deprecated* pre-2018 PSK scheme (incompatible with
  snow — verified against the cacophony vector), and `java-noise` (jchambers) needs Java 17
  and JCA `X25519`, which Android's platform only added in **API 34**. So Android hand-rolls
  the symmetric state + handshake in Kotlin using JCA ChaCha20-Poly1305 (API 28+) and
  HMAC/SHA-256, plus the one pure-Java file `Curve25519.java` vendored from noise-java for
  X25519 (API 29 / Java 8 safe). Do **not** try to assemble it from Tink (§3).
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
- **Prologue: the preamble transcript, canonically framed.** Each side records every byte
  it sends and receives during the plaintext preamble (the whole version exchange,
  including the 8-byte compatibility confirmation when versions differ, then the whole
  mode exchange) and builds

  ```
  prologue = u64_be(len(T_i)) || T_i || u64_be(len(T_r)) || T_r
  ```

  where `T_i` is every byte the **Noise initiator sent** and `T_r` every byte the
  **responder sent**. The initiator computes this as (my sent, my received), the responder
  as (my received, my sent); untampered, both get identical bytes. The length prefixes make
  the encoding unambiguous (no boundary-shifting between the two transcripts). Implemented
  as `build_prologue` / `buildPrologue` next to each Noise implementation, with the
  transcript captured by a recording wrapper at the stream boundary so no branch of the
  negotiation can leak a byte out of the transcript. Per the Noise spec the prologue enters
  only `h` (it gates the handshake MACs, not the derived keys) — that is exactly the
  desired property: mismatch ⇒ handshake abort.
- **PSK derivation:** `PBKDF2-HMAC-SHA256(password_utf8, salt, iters) → 32 bytes`, with
  `salt = b"Flying Carpet v10 shared network PSK"` and `iters = 600000`.
- **Discovery HMAC key:** `discovery_key = HMAC-SHA256(key = psk, data = b"Flying Carpet
  v10 discovery")` — derived from the stretched PSK (never from a fast hash of the
  password; §7, §10), with a fixed label for domain separation so the Noise PSK itself is
  never used outside the handshake. The PSK is derived once, when the password becomes
  known and *before discovery starts*, and reused for the handshake. Implemented as
  `derive_discovery_key` / `deriveDiscoveryKey` next to each PSK derivation.
- **Record framing:** each Noise message is prefixed by its length as a **2-byte
  big-endian** integer (`u16`), tag appended (not prepended). 2 bytes, not 4, because
  Noise caps messages at 65535 bytes; the same framing is used for the two handshake
  messages and every transport record.
- **Roles:** the Noise **initiator is the TCP client**, the **responder is the TCP
  server** — for *both* modes. Shared network: sender = client = initiator, receiver =
  server = responder. Hotspot: the guest that joined and connected = client = initiator,
  the host = server = responder.

**Spec conformance is verified against the official vectors.** `core/src/noise.rs` test
`official_noise_test_vector` drives `snow` with the canonical
`Noise_NNpsk0_25519_ChaChaPoly_SHA256` test vector from
[haskell-cryptography/cacophony](https://github.com/haskell-cryptography/cacophony)
(`vectors/cacophony.txt`) — its prologue, PSK, and fixed ephemerals — and
asserts the handshake message ciphertexts, the handshake hash
(`f4d03dc3…be208eaf`), and the first transport record match byte-for-byte. This is what
guarantees the Swift/Kotlin ports (which follow the same spec) interoperate; **each port
should run the same cacophony vector against its own Noise implementation.** The
app-specific KAT vectors below (PBKDF2-derived PSK, app-style prologue) are a *separate*
determinism check for our exact parameters, not a spec check.

**Cross-platform known-answer test vectors.** These are emitted and asserted by the Rust
reference (`core/src/noise.rs`, tests `psk_known_answer`, `handshake_known_answer`, and
`prologue_known_answer`); Swift and Kotlin must reproduce them exactly. `snow`'s
`fixed_ephemeral_key_for_testing_only` fixes the ephemerals so the whole handshake is
deterministic.

- PSK for password `"flyingcarpet"`:
  `a3d8b7f17f2252e4c2847a365ab2f392beaa996b7e51dd6fa19ff1ad08938619`
- Discovery HMAC key for that PSK:
  `45e49b632788b21069bf48720d6af230ecbd936b3cb16c898a8e1eac51944112`

With fixed PSK = `2a`×32, initiator ephemeral private = `01`×32, responder ephemeral
private = `02`×32, **empty prologue**:

- Handshake msg 1 (initiator → responder), 48 bytes:
  `a4e09292b651c278b9772c569f5fa9bb13d906b46ab68c9df9dc2b4409f8a209a3e9c18456aba2185de800ffaca55b22`
- Handshake msg 2 (responder → initiator), 48 bytes:
  `ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59d887595caf8a0b110dfab84e6b41eafc`
- First transport record from the initiator, plaintext `"hello flying carpet"`:
  `124a00c03b4544f746828bbf9ae2d8d595a9ac1fea988f43f7206c3880180b954f9147`

**Prologue-bound KAT** (same PSK/ephemerals, with the app-style preamble transcript
`T_i` = `000000000000000a` `0000000000000001` (version 10, mode send) and
`T_r` = `000000000000000a` `0000000000000000` (version 10, mode receive)):

- `build_prologue(T_i, T_r)`, 48 bytes:
  `0000000000000010` `000000000000000a0000000000000001` `0000000000000010` `000000000000000a0000000000000000`
- Handshake msg 1: `a4e09292b651c278b9772c569f5fa9bb13d906b46ab68c9df9dc2b4409f8a2093ae03dc8524f79ac9696d6c155df9a3c`
- Handshake msg 2: `ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59d2668070263116ce557500fbe3fd3ba4`
- First transport record (`"hello flying carpet"`):
  `124a00c03b4544f746828bbf9ae2d8d595a9ac1fea988f43f7206c3880180b954f9147` — **identical to
  the empty-prologue record by design**: the prologue enters only `h`, never the chaining
  key, so it changes the handshake message MACs (see msg 1/2 differing after the 32-byte
  ephemeral) but not the transport keys. Asserted anyway so a port that wrongly mixes the
  prologue into `ck` fails the KAT.

(The transport-record vectors are raw Noise messages; on the wire each is preceded by its
2-byte length prefix `0023`.) These are the vectors that guarantee a macOS sender can talk
to an Android receiver; each platform asserts them in its unit tests (Rust
`core/src/noise.rs`, Kotlin `NoiseUnitTest`, Swift `NoiseTests`) since they catch label and
endianness mismatches that otherwise surface as "handshake fails, no idea why." Each
platform also asserts the *negative* cases: a tampered transport record, a tampered
handshake message, a mismatched password, and a mismatched prologue must all fail.

---

## 10. Migration, scope, and locked decisions

**Decisions locked** (from design review):

- **Cipher: ChaCha20-Poly1305.** Noise protocol name **`Noise_NNpsk0_25519_ChaChaPoly_SHA256`**.
  (AES-GCM is also in CryptoKit and would be equally valid; ChaChaPoly chosen as Noise's
  most-tested cipher.)
- **Noise over BOTH modes.** Hotspot mode also runs the Noise handshake now (password =
  the hotspot password, known to both sides), so there is one encryption path for all
  transfers. WPA2 still wraps the hotspot link, but the app no longer relies on it for
  confidentiality.
- **No inner AES.** The old per-chunk AES-256-GCM (keyed by `SHA256(password)`) is
  **removed** — Noise is the sole encryption layer; chunks are raw bytes inside it. The
  `aes-gcm` dependency is gone. `SHA256(password)` survives only for the SSID and the
  discovery HMAC.
- **Plaintext version/mode preamble, bound into the prologue.** Version confirmation and
  send/receive negotiation happen on the raw socket before the handshake (§6, §7), for
  clean version-mismatch reporting — and the full transcript is bound into the Noise
  prologue (§9), so the preamble is readable but not silently modifiable. The binding
  ships in v10 (the first Noise version) because that is the only point at which it can
  close the future downgrade window (§7).
- **No v9 compatibility.** v10 is a clean break; a v9 peer is rejected with a clear message.
- **Discovery HMAC keyed from the stretched PSK** (originally shipped as
  `HMAC(SHA256(password), announcement)`, fixed within v10 before release). The fast-hash
  key made a single captured announcement a cheap offline oracle — crackable within the
  password's live window by a resourced attacker, enabling a real MITM (§7). The
  announcement is now signed with `derive_discovery_key(psk)` (§9), so every oracle
  anywhere in the protocol costs 600k PBKDF2 iterations per guess. Wire-format unchanged
  (same 93 bytes); old and new builds simply never discover each other, which is fine
  within the unreleased v10. `SHA256(password)` now survives *only* for the hotspot SSID.
  **Discovery stays authenticated** — dropping the HMAC (no oracle from announcements) was
  considered and rejected: the handshake's message-1 oracle remains regardless, and the
  HMAC is what gives correct peer selection on a shared LAN (§7). Random per-device names
  as the selection mechanism, with the password as a separate secret, were likewise
  rejected: unauthenticated names are spoofable, so they'd need either the same
  password-derived MAC or a manual verify-the-name step.

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

**The logical send/receive protocol does not change** — only the transport under it (and
the removal of the redundant per-chunk AES). The header-value bounds and filename
sanitization already added remain, applied to the values read from the Noise-decrypted
stream.

---

## 11. Implementation plan (phased)

Order chosen so the cheapest, most-verifiable piece comes first and becomes the reference
the others are tested against.

### Phase 0 — done
Version bump + mismatch messaging (§10). The only pre-Noise code change.

### Phase 1 — Rust core reference implementation (`snow`) — **done**
Landed in `core/src/noise.rs` and wired into `core/src/lib.rs`:
1. `snow` and `pbkdf2` added to `core/Cargo.toml` (and `aes-gcm` removed); protocol
   `Noise_NNpsk0_25519_ChaChaPoly_SHA256`.
2. `derive_psk()` = PBKDF2-HMAC-SHA256 with the fixed salt/iters constants (§9).
3. `start_transfer` runs the **plaintext version/mode preamble** on the raw TCP stream,
   then the Noise handshake, for **both** modes. Noise initiator = TCP client, responder =
   TCP server (§9). A wrong password fails the handshake with a clear message ("Could not
   establish a secure connection. Check that the password matches…").
4. `EncryptedStream<S>` implements tokio `AsyncRead + AsyncWrite`, transparently splitting
   the byte stream into ≤64 KiB Noise records with a 2-byte length prefix. `send_file` /
   `receive_file` / `confirm_version` / `confirm_mode` are generic over the stream; a
   `TransferStream` enum represents the `Plain`→`Encrypted` transition (Plain during the
   preamble, Encrypted after the handshake).
5. **Inner AES removed:** `send_file`/`receive_file` send/receive raw chunks; Noise is the
   sole encryption layer. `SHA256(password)` remains only for the SSID and discovery HMAC.
6. **Spec conformance verified against the official Noise vectors:**
   `official_noise_test_vector` drives `snow` with the canonical cacophony vector for the
   protocol and asserts the message ciphertexts, handshake hash, and transport record
   match byte-for-byte — this is the real cross-platform interop guarantee. Additional
   app-parameter KAT vectors (`psk_known_answer`, `handshake_known_answer`) are transcribed
   into §9 as a determinism check.
7. Verified: `end_to_end_encrypted_transfer` runs the real send/receive over an encrypted
   duplex with a 200 KB (multi-record) file; `wrong_password_fails_handshake`,
   `round_trip_small_and_large`, and `tampering_is_detected` all pass. 19 core tests green.

Phases 2 and 3 must mirror the **full** design: plaintext version/mode preamble → Noise
handshake (both modes) → raw chunks inside Noise (no per-chunk AES). The role rule is the
same everywhere: TCP client = initiator, TCP server = responder.

### Phase 2 — Android (hand-rolled on JCA + vendored X25519) — **done**

**Library dead-end (why hand-rolled).** Neither obvious Java Noise library fits this app
(`minSdk 29`, Java 8, must speak modern `NNpsk0`):
- **rweather/noise-java** implements the *deprecated pre-2018 PSK scheme* (`NoisePSK_`
  prefix, no `psk` token, `SymmetricState` has no `mixKeyAndHash`). Verified empirically:
  it can't parse `Noise_NNpsk0_…` ("Handshake pattern is not recognized"), and its
  `NoisePSK_NN_…` output diverges from the official cacophony `NNpsk0` vector after the
  ephemeral — cryptographically incompatible with the Rust/snow side.
- **jchambers/java-noise** does modern `psk0` but is **Java 17** source and calls JCA
  `KeyAgreement.getInstance("X25519")`, which Android's platform (Conscrypt) only added in
  **Android 14 / API 34** — so it would force `minSdk 34` (dropping everything below Android
  14) or bundling BouncyCastle as a provider. Not acceptable.

So Android hand-rolls the `NNpsk0` symmetric state + handshake + transport in Kotlin
(`Noise.kt`), the same shape as the Apple/CryptoKit side. Only the primitive that isn't in
API-29 platform crypto is vendored:
- **X25519**: the single pure-Java file `com/southernstorm/noise/crypto/Curve25519.java`
  (from rweather/noise-java, MIT; only its *crypto leaf*, not its handshake layer). Works on
  API 29 / Java 8.
- **ChaCha20-Poly1305** (JCA, `Cipher "ChaCha20-Poly1305"`, Android 9 / API 28+),
  **HMAC-SHA256 / SHA-256** (JCA). PSK = the same PBKDF2 as Rust, hand-computed over the
  UTF-8 password so it can't hit `PBEKeySpec` char-encoding ambiguity.

Verified: `NoiseUnitTest` reproduces the official cacophony vector **and** the §9 app KATs
byte-for-byte, plus a multi-record stream round-trip and a wrong-password rejection — 5
tests, matching the Rust reference. Wiring mirrors Phase 1: plaintext version/mode preamble
(already on the raw socket) → `noiseHandshake` for both modes (client = initiator, server =
responder) → the socket streams are swapped for `NoiseInputStream`/`NoiseOutputStream`, and
the per-chunk AES is removed from `Send.kt`/`Receive.kt` (raw chunks). Live Android↔Windows
verification is still pending real devices, but the shared cacophony/app KATs are the
interop guarantee.

### Phase 3 — Apple (hand-rolled on CryptoKit) — **done**
In the FlyingCarpetApple repo (`shared/Noise.swift`), the same shape as the other two:
1. Hand-rolled `NNpsk0` symmetric state + handshake using **CryptoKit** — X25519
   (`Curve25519.KeyAgreement`), ChaCha20-Poly1305 (`ChaChaPoly`), SHA-256, HMAC — and
   **CommonCrypto** PBKDF2 for the PSK (UTF-8 password, identical salt/iters). No
   third-party crypto, honoring the Apple-standard-only constraint.
2. `NoiseConnection` conforms to the existing `TCPConnectionProtocol` (`write` /
   `receiveNBytes`), so after the handshake the transfer code runs unchanged over it; the
   per-chunk AES is removed from `Send.swift`/`Receive.swift` (raw chunks).
3. Wiring (`Transfer.sendAndReceive`): plaintext version/mode preamble → `noiseHandshake`
   for both modes → replace `self.tcp` with the `NoiseConnection`. Role: shared-network
   sender = initiator / receiver = responder; hotspot = initiator (Apple always joins a
   hotspot, never hosts).
4. `NoiseTests` (macOS test target) reproduces the official cacophony vector **and** the §9
   app KATs byte-for-byte, plus a multi-record round-trip and a wrong-password rejection —
   the same vectors Rust and Android assert. Compiles/runs on the developer's Mac (not
   buildable on the Windows dev host); `shared/Noise.swift` must be added to the iOS and
   macOS app targets in Xcode (created outside the IDE).

### Post-review hardening — **done** (all three platforms)
From the code review of Phases 1–3:
1. **Preamble → prologue binding** (§6, §7, §9): every preamble byte, sent and received,
   is recorded by a stream-boundary wrapper (`RecordingStream` /
   `RecordingInputStream`+`RecordingOutputStream` / `RecordingTCPConnection`) and bound
   into the handshake via `build_prologue`/`buildPrologue`. New cross-platform
   `prologue_known_answer` KAT plus a prologue-mismatch negative test on each platform.
   The handshake-failure message now mentions tampering as well as password mismatch.
   **Wire-breaking within the unreleased v10** — all three platforms landed together.
2. **Real tamper tests**: each platform now asserts that a bit-flipped transport record
   and a bit-flipped handshake message fail authentication (the old Rust
   `tampering_is_detected` only exercised the happy path).
3. **Rust: hotspot stored in state before the preamble** (`start_transfer`), so
   `clean_up_transfer` tears the Windows hotspot down even when the version check, mode
   check, or handshake fails (previously those paths left it running until app exit).

### Phase 4 — cross-matrix + cleanup (remaining)
All three platforms now implement the same modern `NNpsk0` and pass the shared cacophony +
app KATs, so they interoperate by construction. **Confirmed on real hardware so far:
Windows↔Android over both hotspot and shared network — before the prologue binding; the
first post-binding live transfer re-confirms it.** Still to do: the rest of the live
matrix (add macOS / iOS × sender / receiver, and Linux), a per-file size larger than one
record end-to-end, and the wrong-password / version-mismatch user-facing paths. The old
cleartext per-chunk AES is already removed on all three platforms. The PSK-derived
discovery key (§9) is implemented in Rust and Android (with the shared KAT); the Apple
repo must mirror it — `deriveDiscoveryKey` via CryptoKit `HMAC<SHA256>`, PSK derived once
at password time (off the main thread) and fed to both discovery and the handshake —
before any v10 release, since the key change is a silent discovery-compat break.

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
7. Know exactly what you're shipping (§7): eavesdroppers read nothing, but both passive and
   active attackers hold a PBKDF2-hardened offline password oracle (discovery announcement
   and first handshake message). It is neutralized by crack cost ≫ single-use password
   lifetime plus forward secrecy, so this is effectively as strong as a PAKE would be *for
   this design* — provided passwords stay single-use and CSPRNG-generated.
