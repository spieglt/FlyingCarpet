// Throughput probe for the v10 encrypted stream. Run with:
//
//     cargo test --release --test throughput -- --ignored --nocapture
//
// Ignored by default: it's a measurement, not an assertion. It exists to answer "is the
// cipher or our framing the reason a big transfer is slow?" without guessing -- every
// number here is off the network, so anything far above the link rate exonerates the code.
use flying_carpet_core::noise::{derive_psk, handshake, Role, NOISE_PARAMS};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BYTES: usize = 512 * 1024 * 1024; // 512 MB
const CHUNK: usize = 1_000_000; // CHUNKSIZE in lib.rs

fn rate(label: &str, bytes: usize, seconds: f64) {
    let mbytes = bytes as f64 / 1_048_576.0;
    let mbits = 8.0 * (bytes as f64 / 1_000_000.0);
    println!(
        "{:<38} {:>8.1} MB/s  ({:>9.0} mbps, {:.2}s for {:.0} MiB)",
        label,
        mbytes / seconds,
        mbits / seconds,
        seconds,
        mbytes
    );
}

// The cipher alone: ChaCha20-Poly1305 through snow's transport state, at the record size
// EncryptedStream actually uses. No framing, no sockets, no allocation per record beyond
// what snow does internally.
#[test]
#[ignore]
fn cipher_only_throughput() {
    let psk = [0x2a; 32];
    let params: snow::params::NoiseParams = NOISE_PARAMS.parse().unwrap();
    let mut initiator = snow::Builder::new(params.clone())
        .psk(0, &psk)
        .unwrap()
        .build_initiator()
        .unwrap();
    let mut responder = snow::Builder::new(params)
        .psk(0, &psk)
        .unwrap()
        .build_responder()
        .unwrap();
    let mut buf = vec![0u8; 65535];
    let mut tmp = vec![0u8; 65535];
    let n = initiator.write_message(&[], &mut buf).unwrap();
    responder.read_message(&buf[..n], &mut tmp).unwrap();
    let n = responder.write_message(&[], &mut buf).unwrap();
    initiator.read_message(&buf[..n], &mut tmp).unwrap();
    let mut sender = initiator.into_transport_mode().unwrap();
    let mut receiver = responder.into_transport_mode().unwrap();

    let plaintext = vec![0xabu8; 65519]; // MAX_PLAINTEXT
    let records = BYTES / plaintext.len();

    // Encrypt-only first. Every record has to be read by `receiver` in the same order it
    // was written or the nonce counters drift apart, so this pass feeds the decrypt loop
    // below rather than throwing its output away.
    let mut records_out: Vec<Vec<u8>> = Vec::with_capacity(records);
    let start = Instant::now();
    for _ in 0..records {
        let n = sender.write_message(&plaintext, &mut buf).unwrap();
        records_out.push(buf[..n].to_vec());
    }
    rate("encrypt only (snow transport)", records * plaintext.len(), start.elapsed().as_secs_f64());

    let start = Instant::now();
    for record in &records_out {
        receiver.read_message(record, &mut tmp).unwrap();
    }
    rate("decrypt only (snow transport)", records * plaintext.len(), start.elapsed().as_secs_f64());
}

// The whole EncryptedStream, in memory. Isolates our framing, buffering and copies from
// both the cipher and the network: whatever this number is, the real transfer cannot beat
// it, and the gap between this and `cipher_only` is what our stream layer costs.
#[test]
#[ignore]
fn encrypted_stream_in_memory_throughput() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (a, b) = tokio::io::duplex(4 * 1024 * 1024);
        let psk = derive_psk("benchmark password");
        let prologue = Vec::new();
        let p2 = prologue.clone();

        let ta = tokio::spawn(async move { handshake(a, Role::Initiator, &psk, &prologue).await });
        let tb = tokio::spawn(async move { handshake(b, Role::Responder, &psk, &p2).await });
        let mut sender = ta.await.unwrap().unwrap();
        let mut receiver = tb.await.unwrap().unwrap();

        let chunk = vec![0xcdu8; CHUNK];
        let chunks = BYTES / CHUNK;

        let reader = tokio::spawn(async move {
            let mut sink = vec![0u8; CHUNK];
            for _ in 0..chunks {
                receiver.read_exact(&mut sink).await.unwrap();
            }
        });

        let start = Instant::now();
        for _ in 0..chunks {
            // mirrors sending.rs: an 8-byte length then the chunk body
            sender.write_u64(CHUNK as u64).await.unwrap();
            sender.write_all(&chunk).await.unwrap();
        }
        sender.flush().await.unwrap();
        reader.await.unwrap();
        rate("EncryptedStream over duplex (in-memory)", chunks * CHUNK, start.elapsed().as_secs_f64());
    });
}

// What sizes actually hit the socket for one chunk. Loopback and duplex both hide the cost
// of a small write -- a real link does not, because Nagle will hold a sub-MSS segment until
// the peer ACKs, and a receiver with nothing to send back delays that ACK by tens to
// hundreds of milliseconds. One such stall per chunk is the difference between a fast
// transfer and a crawl, so this records the shape rather than the speed.
#[test]
#[ignore]
fn socket_write_pattern_for_one_chunk() {
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    // Records the size of every write that reaches the inner stream.
    struct Counting<S> {
        inner: S,
        sizes: Arc<Mutex<Vec<usize>>>,
    }
    impl<S: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for Counting<S> {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            let this = self.get_mut();
            let res = Pin::new(&mut this.inner).poll_write(cx, buf);
            if let Poll::Ready(Ok(n)) = &res {
                this.sizes.lock().unwrap().push(*n);
            }
            res
        }
        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            let this = self.get_mut();
            Pin::new(&mut this.inner).poll_flush(cx)
        }
        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            let this = self.get_mut();
            Pin::new(&mut this.inner).poll_shutdown(cx)
        }
    }
    impl<S: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for Counting<S> {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let this = self.get_mut();
            Pin::new(&mut this.inner).poll_read(cx, buf)
        }
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (a, b) = tokio::io::duplex(8 * 1024 * 1024);
        let psk = derive_psk("benchmark password");
        let tb = tokio::spawn(async move { handshake(b, Role::Responder, &psk, &[]).await });
        let sizes = Arc::new(Mutex::new(Vec::new()));
        let counted = Counting { inner: a, sizes: Arc::clone(&sizes) };
        let mut sender = handshake(counted, Role::Initiator, &psk, &[]).await.unwrap();
        let mut receiver = tb.await.unwrap().unwrap();

        let reader = tokio::spawn(async move {
            let mut sink = vec![0u8; CHUNK];
            receiver.read_exact(&mut sink).await.unwrap();
        });

        sizes.lock().unwrap().clear(); // drop the handshake writes
        sender.write_u64(CHUNK as u64).await.unwrap(); // the length prefix
        sender.write_all(&vec![0xcdu8; CHUNK]).await.unwrap(); // the chunk body
        sender.flush().await.unwrap();
        reader.await.unwrap();

        let sizes = sizes.lock().unwrap();
        println!("socket writes for one {}-byte chunk: {} total", CHUNK, sizes.len());
        println!("  first:  {} bytes  <- length prefix, its own segment", sizes[0]);
        println!("  rest:   {} writes of {} bytes", sizes.len() - 1, sizes[1]);
        let small = sizes.iter().filter(|n| **n < 1500).count();
        println!("  sub-MSS writes (what Nagle holds): {}", small);
    });
}

// Same, but over a real loopback TCP socket, so the per-record syscalls and the read-side
// 8 KiB cap are in play. Closest thing to the transfer path without a network.
#[test]
#[ignore]
fn encrypted_stream_loopback_tcp_throughput() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let psk = derive_psk("benchmark password");

        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            handshake(sock, Role::Responder, &psk, &[]).await.unwrap()
        });
        let client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut sender = handshake(client, Role::Initiator, &psk, &[]).await.unwrap();
        let mut receiver = server.await.unwrap();

        let chunk = vec![0xcdu8; CHUNK];
        let chunks = BYTES / CHUNK;

        let reader = tokio::spawn(async move {
            let mut sink = vec![0u8; CHUNK];
            for _ in 0..chunks {
                receiver.read_exact(&mut sink).await.unwrap();
            }
        });

        let start = Instant::now();
        for _ in 0..chunks {
            sender.write_u64(CHUNK as u64).await.unwrap();
            sender.write_all(&chunk).await.unwrap();
        }
        sender.flush().await.unwrap();
        reader.await.unwrap();
        rate("EncryptedStream over loopback TCP", chunks * CHUNK, start.elapsed().as_secs_f64());
    });
}
