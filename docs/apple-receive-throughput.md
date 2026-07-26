# v10 transfers ran at 43 mbps on a 900 mbps link (RESOLVED, 2026-07-25)

**Cause: the Rust sender was built in debug mode.** `snow`'s ChaCha20-Poly1305 runs at
~13.8 MB/s at `opt-level = 0` versus ~291 MB/s in release, and `EncryptedStream::poll_write`
encrypts *inside* the write call — so the sender was CPU-bound at ~5 MB/s while appearing, to
its own instrumentation, to be "blocked on the socket". A release build moved the same
transfer from **43 mbps to 907.8 mbps**, matching iperf3's 907 mbps ceiling for this link.

Nothing on the Apple side was at fault. The receiver was idle 95% of every transfer.

**Fix on the Rust side:**
- Build release for any throughput testing, and
- `[profile.dev.package."*"] opt-level = 3` in the workspace `Cargo.toml`, so dev builds
  optimize dependencies while keeping app code debuggable.
- Fix the sender's `Diag:` line before deleting it: it reports time in `write_all` as
  "socket", but that call encrypts too. **That single mislabel sent this entire investigation
  to the wrong machine.**

## What this cost, and the lesson

Three confident theories about the Swift receiver were each disproven by the next
measurement:

| theory | disproved by |
|---|---|
| Too many async round trips per 64KB record | Cutting receives per MB 8x moved throughput 0.7% |
| The TCP window collapses between on-demand reads | `NWConnection` hit 892 mbps with 45KB reads, 2400/sec |
| 64KB Noise records are too small | Prefetching whole 5MB chunks (153 receives -> 2) moved it 0% |

The receiver was never measured directly until late; every number came from the sender,
whose timer conflated CPU with I/O. **Measure the machine you suspect, and distrust any timer
wrapping a call that does both compute and I/O.**

## Symptom (as originally reported)

Windows (Rust sender) → macOS (Swift receiver), shared network mode, 4.58 GB file:
**~43 mbps (5.35 MB/s)**, where SMB moved the same file, from the same disk, over the same
network, to the same MacBook at ~600 mbps. Later measured directly: a single plain TCP stream
over that link does **907 mbps**.

## What the measurements say

A temporary diagnostic in the Rust sender (`core/src/sending.rs`) splits the send loop's
wall time into disk-wait and socket-wait, reported every 5s:

```
Diag: 43.2mbps over last 5s — 27 chunks: disk 0.2ms/chunk, socket 185.0ms/chunk   (1MB chunks)
Diag: 42.8mbps over last 6s —  6 chunks: disk 1.0ms/chunk, socket 933.5ms/chunk   (5MB chunks)
```

- **The sender is blocked on the socket ~100% of the time.** It is not the network and not
  the sender: TCP flow control is throttling it because the Mac drains at 5.35 MB/s.
- **The disk is irrelevant** (0.2-1.0 ms/chunk; the file is in the page cache).
- **The limit is per byte, not per chunk.** 5x the chunk size gave 5x the time per chunk
  and *identical* throughput. So chunk size is not the lever.

## Ruled out, with the evidence

| Hypothesis | Verdict |
|---|---|
| ChaCha20-Poly1305 slower than v9's AES-GCM | **This was the right neighbourhood and was dismissed too fast.** The benchmark below was run with `cargo test --release`, so it never exercised the debug build the app actually ran. `core/tests/throughput.rs` measures 968 MB/s encrypt, 1633 MB/s decrypt, and 769 MB/s through the whole Rust `EncryptedStream` over loopback TCP — 150x the observed rate. |
| Nagle's algorithm on the sender | No. The send loop writes a small length prefix then immediately queues the body, so Nagle coalesces rather than stalling; it needs a write-write-*read* pattern. `set_nodelay(true)` was added anyway (correct for this protocol) and changed nothing. |
| Chunk size | No. See above — 1 MB and 5 MB give the same throughput. |
| Sender's spinning disk | No. 0.2 ms per 1 MB read. |
| macOS app built in Debug | No. A Release build measured the same, which also rules out Swift-level compute cost and points at per-call latency. **Note the irony: the *receiver's* build mode was checked and cleared, the *sender's* was never asked about. That was the answer.** |

## Mechanism — WRONG, kept as a record of the wrong turn

*Everything in this section was disproven. The reasoning is plausible and the code reading is
accurate; the conclusion is not. Reducing these round trips 8x changed throughput by 0.7%.*

### The (incorrect) claim: a v10 regression introduced with Noise

v9 read a whole chunk in one call, so Network.framework could deliver up to 5 MB per
callback:

```swift
// v9, Apple/shared/Receive.swift (commit 6996b38~1)
receiveNBytes(n: chunkSize)   // -> receive(minimumIncompleteLength: 0, maximumLength: 5_000_000)
```

v10 routes the same read through `NoiseConnection`, which pulls **one 64 KB Noise record at
a time**, and every record costs two async receives — `Apple/shared/Noise.swift:333`:

```swift
private func readNoiseFrame(_ tcp: any TCPConnectionProtocol) async throws -> Data {
    let lenBytes = try await tcp.receiveNBytes(n: 2)   // asks Network.framework for 2 bytes
    let len = (Int(b[0]) << 8) | Int(b[1])
    return try await tcp.receiveNBytes(n: len)         // capped at 65535
}
```

Two things got worse at once:

1. **Max delivery per callback fell from 5,000,000 bytes to 65,535.**
2. **A mandatory 2-byte round trip was added per record.**

A 4.58 GB file is ~70,000 records, so at minimum ~140,000 async receives where v9 needed a
few thousand. It is worse than that in practice because `receiveUpToNBytes` (as it then
was, in `Apple/shared/Network.swift`) passed `minimumIncompleteLength: 0`, which let each call
return as little as a single TCP segment — 3M+ receives for this file.

This is per byte (records are 64 KB regardless of chunk size), which is exactly what the
measurements show.

The Rust receiver does structurally similar small reads (8 KiB per `poll_read` in
`EncryptedStream`) but they are ordinary syscalls at ~1-2 µs, not dispatch hops with
checked continuations — hence Rust→Rust is unaffected.

## What was changed on the Apple side (kept, but not the fix)

Local to Swift, no wire change, no KAT impact, no coordination with the Rust or Kotlin
ports. `NoiseConnection` already buffered *plaintext* above the framing; what was missing
was the equivalent buffering of *ciphertext* underneath it.

1. **`BufferedCiphertextReader`** (`Apple/shared/Noise.swift`) sits between `NWConnection` and the
   Noise framing. It pulls `NOISE_SOCKET_BLOCK` (256 KB) at a time and serves the 2-byte
   lengths and 64 KB record bodies out of memory, so ~4 records cost one socket read instead
   of 8. `noiseHandshake` wraps the connection once and hands that same instance to the
   `NoiseConnection` it returns — a handshake read may pull transport bytes in with it, and
   they would be stranded if the transport then read from the bare connection.
2. **`minimumIncompleteLength` is now the caller's real minimum**, not 0
   (`TCPConnectionProtocol.receiveSome` in `Apple/shared/Network.swift`, replacing
   `receiveUpToNBytes`). Every read here is of a protocol field whose length is already
   known, so the transport is asked for the whole remainder and delivers it in one callback
   instead of one per TCP segment. The minimum is never more than what the peer already
   owes — this protocol has points where the peer is waiting on our reply, so over-asking
   would deadlock.
   - `receiveSome` became a protocol *requirement* rather than an extension-only method.
     The wrappers (`RecordingTCPConnection`, `BufferedCiphertextReader`, `NoiseConnection`)
     have to serve reads from their own state; as an extension method it dispatched
     statically and a caller holding a wrapper would read straight off the socket, around
     the buffer or the preamble transcript.
3. **No O(remaining) drains.** The ciphertext buffer uses an offset that compacts once per
   refill. Above the framing, `NoiseConnection.receiveSome` accumulates decrypted records
   directly into the buffer it returns and keeps at most one record's tail, so a 5 MB chunk
   read no longer copies 5 MB out of a staging buffer and then memmoves the remainder.

Also on the send side: `NoiseConnection.write` coalesces records into 256 KB batches, so a
5 MB chunk costs 20 awaited sends instead of 77. Same framing on the wire — only the number
of `NWConnection.send` calls changes.

**This did not fix the bug** (43.2 → 43.5 mbps on hardware). It is kept because it is
correct on its own terms and measures 1.24x on loopback, and because it makes the receive
count a variable we can now rule out. `NoiseTransportBufferingTests` covers it: transparency
across delivery sizes, unaligned reads spanning record boundaries, unchanged send framing,
and ~4 socket receives per megabyte instead of 32+.

## The receiver instrumentation (since removed)

While this was open, `Apple/shared/Receive.swift` emitted the counterpart to the Rust sender's
line every 5s. It has been removed now that the bug is closed; the patch is recoverable from
the session scratchpad (`instrumentation.patch`) if it is ever wanted again:

```
Diag: 43.5mbps over last 5s — 6 chunks: socket 900.0ms, decrypt 8.0ms, disk 1.4ms, other 2.0ms per chunk; 20 socket reads averaging 256KB
```

`socket` is time blocked inside `NWConnection.receive`; `decrypt` is time inside
`ChaChaPoly.open`; `disk` is `seekToEnd` + `write`. The averaging figure is how much each
receive actually returned.

## What the receiver actually reports (2026-07-25, 4.58 GB Windows → macOS)

```
Diag: 43.6mbps over last 6s — 6 chunks: socket 874.8ms, decrypt 33.9ms, disk 2.3ms, other 7.0ms per chunk; 918 socket reads averaging 32KB
Diag: 43.2mbps over last 6s — 6 chunks: socket 884.9ms, decrypt 30.8ms, disk 3.5ms, other 7.6ms per chunk; 913 socket reads averaging 32KB
```

**The Mac is idle 95% of the time.** Decrypt 34 ms, disk 2-3 ms, our own copying 7 ms — 44 ms
of work per 5 MB chunk against 875 ms of waiting. Every candidate inside this app is now
measured and none of them is the bottleneck.

The shape of the waiting: **153 reads per chunk, 5.7 ms blocked each, 32 KB returned each**,
where 256 KB was asked for. Blocked-then-32KB means the kernel buffer was *empty* when we
asked — the receiver is starved, not busy. 32 KB per 5.7 ms is 5.6 MB/s, which is the entire
deficit, and it is the signature of a window-over-RTT limit.

Both ends are now known to be blocked: the sender in `write_all`, the receiver in `receive`.
That puts the bottleneck between them — the path, or TCP flow control over it — not in
either program's compute.

The link is not the suspect: SMB moves 600 mbps between these two machines over the same
path, and the slowness is not specific to shared network mode (it was chosen for testing
precisely because it is the *faster* path and gives SMB as a control).

## What v9 actually did differently

From the v9 source (`Apple/shared/Receive.swift` at commit d5c018d):

```swift
chunkBytes = try await tcp.receiveNBytes(n: chunkSize)  // one request for the whole 5MB
...                                                      // nothing but append() in the loop
return try AES.GCM.open(sealedBox, using: key)          // one decrypt of all 5MB
```

Two things changed in v10, and the measurements say only the first one matters:

1. **The largest outstanding receive fell from 5,000,000 bytes to 65,535.** v9 asked for a
   whole chunk (`maximumLength` = the full remainder) and did nothing but append between
   calls. v10 routes the read through the Noise framing, which asks for a 2-byte length and
   then a ≤64 KB body. Since `NWConnection` drains the kernel socket only while a receive is
   outstanding, v10 leaves it unattended between every record. **This is the regression.**
2. **Decrypts per chunk went from 1 to 77** (5 MB AES-GCM → 64 KB ChaChaPoly records). Real,
   but it costs 34 ms of a 920 ms chunk. Making decryption *free* would take 43 mbps to
   ~46 mbps. Neither the record granularity nor the AES-GCM → ChaCha20 change is the bug.

### The fix

`BufferedCiphertextReader.prefetch(atLeast:)`, called by `NoiseConnection.receiveSome`
before it reads a chunk's worth of records. The receiver knows the chunk length before the
chunk arrives, so it can safely block on one large receive for the whole thing — v9's shape,
restored under v10's framing, with no wire change. The bound passed down is a deliberate
underestimate of the ciphertext still coming (`n + 18 × floor(n / 65519)`, since each record
costs a 2-byte length and a 16-byte tag); asking for more than the peer owes would block
forever.

### Result: it worked, and the throughput did not move

```
Diag: 43.3mbps over last 6s — 6 chunks: socket 907.1ms, decrypt 13.8ms, disk 1.1ms, other 1.4ms per chunk; 15 socket reads averaging 1958KB
Diag: 43.7mbps over last 5s — 6 chunks: socket 895.5ms, decrypt 14.6ms, disk 1.8ms, other 3.6ms per chunk; 12 socket reads averaging 2442KB
```

Reads per chunk fell from **153 averaging 32 KB** to **2.5 averaging 2 MB**, exactly as
intended. Throughput: **43.3 mbps, unchanged.** The Mac now asks for a whole chunk in
essentially one receive and still waits 907 ms for it.

**This exonerates the receiver.** Bytes arrive at 5.5 MB/s regardless of how they are asked
for, so no change to the receive path can matter. Keep the prefetch anyway — it more than
halved the per-chunk CPU overhead (decrypt 34→14 ms, other 7→1.4 ms) and restores v9's shape
— but it is not the fix for this bug either.

**It also retires the read-ahead pump.** Processing is now 17 ms of a 920 ms chunk, so
keeping a receive posted during it is worth ≤2%. Not worth ~100 lines of concurrency; the
parked copy should be deleted rather than kept.

## Where the bug is not

Everything on the receiving Mac is now measured and cleared:

| | measured | needed to explain 43 mbps |
|---|---|---|
| crypto + framing + `NWConnection` (loopback) | 690 MB/s | 5.4 MB/s |
| disk write pattern | 3570 MB/s | 5.4 MB/s |
| decrypt, in situ | 14 ms per 5 MB chunk | 920 ms |
| disk, in situ | 1.1 ms per chunk | 920 ms |
| read pattern | 2 MB per receive | — |
| Wi-Fi link (802.11ax, 6 GHz, 160 MHz, −60 dBm) | 1441 mbps PHY | 43 mbps |

Both ends are blocked at once — the sender in `write_all`, the receiver in `receive` — with a
healthy radio on this end. That is a path or sender-side limit.

## The network is fine, and so is Network.framework

| receiver | result |
|---|---|
| iperf3 (BSD sockets) | **907 mbps** |
| `NWConnection`, with this app's TCP options, 45 KB average reads | **892 mbps** |
| Flying Carpet | 43 mbps |

Same Windows sender, same link, same direction, minutes apart. Note the middle row's read
size: 45 KB per receive, 2400 receives a second, and it still saturates the link. Read size
and read pattern were never the problem — 32 KB reads were a *symptom* of nothing arriving.
(The `nwrecv` probe used for this is in the session scratchpad, not the repo.)

## Where it actually points: the sender encrypts inside `write_all`

`EncryptedStream::poll_write` (`core/src/noise.rs`) encrypts on the
write call:

```rust
let take = buf.len().min(MAX_PLAINTEXT);
let n = this.noise.write_message(&buf[..take], &mut msg)?;   // ChaCha20-Poly1305, here
```

So the sender's `Diag:` line, which reports time in `write_all` as **"socket"**, is really
reporting *encrypt + socket*. "Blocked in write_all ~100% of the time" never meant blocked on
the network. **That mislabel is what pointed this entire investigation at the Apple side.**

`core/Cargo.toml` has `snow = "0.10"` with default features — the pure-Rust
`chacha20poly1305` backend — and no `[profile.*]` overrides anywhere in the workspace, so a
debug build runs it at `opt-level = 0`. Measured standalone on an M-series Mac
(`scratchpad/snowbench`):

| cipher | debug | release |
|---|---|---|
| snow ChaCha20-Poly1305 (v10) | **13.8 MB/s** | 291 MB/s |
| AES-256-GCM (v9's cipher) | **6.5 MB/s** | 213 MB/s |

The transfer moves 5.4 MB/s. That is the debug column's order of magnitude and ~50x below
the release column. Note also that hardware AES does *not* survive a debug build — v9's
cipher measures slower than v10's at `opt-level = 0` — so the v9→v10 regression is not
explained by the cipher change. If v9 was measured against a shipped release binary and v10
against a dev build, the build profile explains it by itself.

The clincher is inside a single v10 transfer: **the Mac decrypts at 357 MB/s** (14 ms per
5 MB chunk, via CryptoKit, always optimized) **while the sender encrypts at 5.4 MB/s.** Same
algorithm, same bytes, 66x apart.

### What to check, and the fix

1. **How is the Windows app built?** `npm run tauri dev` / `cargo run` without `--release`
   builds the core in debug. Re-run the transfer from a release build; if it jumps, done.
2. **Durable fix either way** — in the workspace `Cargo.toml`, optimize
   dependencies even in dev builds, keeping the app's own code debuggable:

   ```toml
   [profile.dev.package."*"]
   opt-level = 3
   ```

   Crypto crates are exactly the case this profile setting exists for.
3. **Fix the sender's `Diag:` label** before removing it: split time in `write_all` into
   encrypt and socket, or rename it. As written it attributes CPU time to the network.

The earlier throughput benchmark that "ruled out" the cipher at 968 MB/s was run with
`cargo test --release`, so it never exercised the build the app actually ships in dev.

## Superseded: is a single TCP flow between these machines worth more than 43 mbps?

Answered above — yes, 907 mbps. Kept for the method.

The SMB comparison has a hole worth checking: **SMB multichannel opens several TCP
connections**, so 600 mbps aggregate is consistent with a per-flow limit near 43-75 mbps. If
a single plain TCP stream also lands at ~43 mbps, no application change on either side will
help, and the answer is in the network (loss, retries, the Windows adapter or its TCP
settings) — not in this repo or the Rust core.

Zero-install test, Mac as receiver (its address was 192.168.86.226 when this was written):

```
# on the Mac
nc -l 5001 > /dev/null
```
```powershell
# on Windows: 500 MB over one TCP connection, no Flying Carpet, no crypto
$c = New-Object System.Net.Sockets.TcpClient('192.168.86.226', 5001)
$s = $c.GetStream(); $buf = New-Object byte[] 1048576
$sw = [Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt 500; $i++) { $s.Write($buf, 0, $buf.Length) }
$s.Close(); $c.Close(); $sw.Stop()
"{0:N1} mbps" -f (500 * 8 / $sw.Elapsed.TotalSeconds)
```

Or with iperf3 (already installed on the Mac), which also answers the multichannel question
directly — `iperf3 -s` on the Mac, then on Windows:

```
iperf3 -c 192.168.86.226 -t 20        # one stream: compare against 43 mbps
iperf3 -c 192.168.86.226 -t 20 -P 8   # eight streams: compare against SMB's 600 mbps
```

- **One stream ≈ 43 mbps** → the app is not involved; it is the path or the Windows host.
  Eight streams ≈ 600 would confirm SMB's number came from aggregation.
- **One stream ≈ 600 mbps** → the network is fine for a single flow, and the remaining
  suspect is the Rust sender's socket behavior.

The Apple-side instrumentation has been removed. **The sender's `Diag:` line still needs
fixing or removing** — see the header.

### Still to check

iOS, and the reverse direction (macOS → Windows), which share this code. Both are sanity runs
now rather than investigations.

## Related pending changes

FlyingCarpet (Rust/Android):
- `core/src/lib.rs` — `set_nodelay(true)`; `chunksize()` reading `FC_CHUNKSIZE` (temporary,
  for the chunk-size experiment above).
- `core/src/sending.rs` — the temporary `Diag:` instrumentation. **Remove before release.**
- `core/tests/throughput.rs` — ignored benchmarks (cipher, stream, socket write pattern).
  Run with `cargo test --release --test throughput -- --ignored --nocapture`.
- `Android/.../MainViewModel.kt` — `client.tcpNoDelay = true`.
