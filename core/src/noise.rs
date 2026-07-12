// Encrypted transport for all transfers (v10+): both hotspot and shared network mode.
//
// Wraps the TCP connection in a Noise `NNpsk0` handshake and record layer so that file
// contents AND metadata (filenames, sizes, counts, hashes) are confidential and
// tamper-evident, with forward secrecy from ephemeral X25519 keys. The password is fed in
// as the Noise pre-shared key. This is the sole encryption layer — there is no separate
// application-level chunk cipher. The handshake runs after the plaintext version/mode
// preamble (see start_transfer). See docs/shared-network-crypto.md for the full design and
// threat model; this is the Rust reference implementation the Swift and Kotlin ports are
// tested against.
//
// EncryptedStream implements tokio's AsyncRead + AsyncWrite, transparently splitting the
// byte stream into <=64 KiB Noise transport messages, so the existing send/receive code
// runs over it unchanged.

use crate::error::FCError;
use std::io;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

// Must be byte-identical across Rust, Swift, and Kotlin.
pub const NOISE_PARAMS: &str = "Noise_NNpsk0_25519_ChaChaPoly_SHA256";
// Fixed domain-separation salt for PBKDF2. Not secret; the Noise handshake hash already
// binds the ephemeral transcript, so the salt's only job is domain separation.
pub const PSK_SALT: &[u8] = b"Flying Carpet v10 shared network PSK";
pub const PBKDF2_ITERS: u32 = 600_000;

// Noise transport messages are capped at 65535 bytes including the 16-byte AEAD tag.
const NOISE_TAG_LEN: usize = 16;
const MAX_NOISE_MESSAGE: usize = 65535;
const MAX_PLAINTEXT: usize = MAX_NOISE_MESSAGE - NOISE_TAG_LEN; // 65519

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Initiator, // shared network sender (TCP client)
    Responder, // shared network receiver (TCP server)
}

/// Derives the 32-byte Noise pre-shared key from the transfer password.
/// PBKDF2-HMAC-SHA256 with a fixed salt and iteration count (constants above); the
/// stretching only slows the one residual attack (an active in-path attacker's offline
/// guess), so the parameters must match across platforms but need not be aggressive.
pub fn derive_psk(password: &str) -> [u8; 32] {
    use pbkdf2::pbkdf2_hmac;
    use sha2::Sha256;
    let mut psk = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), PSK_SALT, PBKDF2_ITERS, &mut psk);
    psk
}

/// Builds the canonical Noise prologue from the plaintext preamble transcript.
/// `initiator_transcript` is every byte the Noise initiator sent during the preamble
/// (version + mode exchange) and `responder_transcript` every byte the responder sent;
/// each is length-prefixed (u64 big-endian, matching the app's length idiom) so the
/// encoding is unambiguous. Each side computes this from its own sent/received bytes —
/// the initiator as (sent, received), the responder as (received, sent) — so any
/// in-flight tampering with the preamble makes the prologues differ, which fails the
/// handshake. Must be byte-identical across Rust, Swift, and Kotlin.
pub fn build_prologue(initiator_transcript: &[u8], responder_transcript: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(16 + initiator_transcript.len() + responder_transcript.len());
    p.extend_from_slice(&(initiator_transcript.len() as u64).to_be_bytes());
    p.extend_from_slice(initiator_transcript);
    p.extend_from_slice(&(responder_transcript.len() as u64).to_be_bytes());
    p.extend_from_slice(responder_transcript);
    p
}

/// Runs the Noise NNpsk0 handshake over `inner` and returns an EncryptedStream on success.
/// `prologue` binds the plaintext preamble transcript (see build_prologue): if either the
/// password or the preamble bytes differ between the peers, the first authenticated
/// message fails to decrypt — surfaced as a clear, user-facing error.
pub async fn handshake<S>(
    mut inner: S,
    role: Role,
    password: &str,
    prologue: &[u8],
) -> Result<EncryptedStream<S>, FCError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let psk = derive_psk(password);
    let params = NOISE_PARAMS.parse().map_err(|e| FCError {
        message: format!("Invalid Noise parameters: {}", e),
    })?;
    let builder = snow::Builder::new(params)
        .prologue(prologue)
        .map_err(|e| FCError {
            message: format!("Noise builder error: {}", e),
        })?
        .psk(0, &psk)
        .map_err(|e| FCError {
            message: format!("Noise builder error: {}", e),
        })?;
    let mut hs = match role {
        Role::Initiator => builder.build_initiator(),
        Role::Responder => builder.build_responder(),
    }
    .map_err(|e| FCError {
        message: format!("Noise handshake init error: {}", e),
    })?;

    let mut buf = vec![0u8; MAX_NOISE_MESSAGE];
    match role {
        Role::Initiator => {
            // -> psk, e
            let n = hs.write_message(&[], &mut buf).map_err(|e| FCError {
                message: format!("Noise write error: {}", e),
            })?;
            write_frame(&mut inner, &buf[..n]).await?;
            // <- e, ee
            let msg = read_frame(&mut inner).await?;
            hs.read_message(&msg, &mut buf).map_err(|_| FCError {
                message: "Could not establish a secure connection. Check that the password matches on both devices. (This can also mean the connection was tampered with.)".to_string(),
            })?;
        }
        Role::Responder => {
            // -> psk, e
            let msg = read_frame(&mut inner).await?;
            hs.read_message(&msg, &mut buf).map_err(|_| FCError {
                message: "Could not establish a secure connection. Check that the password matches on both devices. (This can also mean the connection was tampered with.)".to_string(),
            })?;
            // <- e, ee
            let n = hs.write_message(&[], &mut buf).map_err(|e| FCError {
                message: format!("Noise write error: {}", e),
            })?;
            write_frame(&mut inner, &buf[..n]).await?;
        }
    }

    let transport = hs.into_transport_mode().map_err(|e| FCError {
        message: format!("Noise transport error: {}", e),
    })?;
    Ok(EncryptedStream::new(inner, transport))
}

// Handshake messages use the same u16-length framing as the record layer.
async fn write_frame<S: AsyncWrite + Unpin>(inner: &mut S, msg: &[u8]) -> Result<(), FCError> {
    let len = u16::try_from(msg.len()).map_err(|_| FCError {
        message: "Noise handshake message too large".to_string(),
    })?;
    inner.write_all(&len.to_be_bytes()).await?;
    inner.write_all(msg).await?;
    inner.flush().await?;
    Ok(())
}

async fn read_frame<S: AsyncRead + Unpin>(inner: &mut S) -> Result<Vec<u8>, FCError> {
    let mut len_buf = [0u8; 2];
    inner.read_exact(&mut len_buf).await?;
    let len = u16::from_be_bytes(len_buf) as usize;
    let mut msg = vec![0u8; len];
    inner.read_exact(&mut msg).await?;
    Ok(msg)
}

/// Wraps the raw stream during the plaintext preamble, recording every byte sent and
/// received so the transcript can be bound into the Noise prologue (see build_prologue).
/// Recording at the stream boundary — rather than inside the preamble functions — means
/// no exchanged byte can be missed, whatever branch the version/mode negotiation takes.
pub struct RecordingStream<S> {
    inner: S,
    sent: Vec<u8>,
    received: Vec<u8>,
}

impl<S> RecordingStream<S> {
    pub fn new(inner: S) -> Self {
        RecordingStream {
            inner,
            sent: Vec::new(),
            received: Vec::new(),
        }
    }

    /// Returns the inner stream and the (sent, received) transcripts.
    pub fn into_parts(self) -> (S, Vec<u8>, Vec<u8>) {
        (self.inner, self.sent, self.received)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for RecordingStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let res = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &res {
            this.received.extend_from_slice(&buf.filled()[before..]);
        }
        res
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for RecordingStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &res {
            this.sent.extend_from_slice(&buf[..*n]);
        }
        res
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

enum ReadState {
    Len,  // reading the 2-byte length prefix
    Body, // reading the record body
}

/// A byte-stream view over a Noise transport session: bytes written are encrypted into
/// length-prefixed Noise records; bytes read are decrypted from them. Implements tokio's
/// AsyncRead + AsyncWrite so the transfer code uses it exactly like a TcpStream.
pub struct EncryptedStream<S> {
    inner: S,
    noise: snow::TransportState,
    // write side: at most one framed record is buffered; poll_write only reports bytes
    // accepted once that record is fully written to `inner`, so nothing is left buffered
    // across a write->read boundary (which would deadlock the alternating protocol).
    write_buf: Vec<u8>,
    write_pos: usize,
    pending_take: usize,
    // read side
    read_state: ReadState,
    read_need: usize,
    read_acc: Vec<u8>,
    read_plain: Vec<u8>,
    read_plain_pos: usize,
}

impl<S> EncryptedStream<S> {
    fn new(inner: S, noise: snow::TransportState) -> Self {
        EncryptedStream {
            inner,
            noise,
            write_buf: Vec::new(),
            write_pos: 0,
            pending_take: 0,
            read_state: ReadState::Len,
            read_need: 2,
            read_acc: Vec::new(),
            read_plain: Vec::new(),
            read_plain_pos: 0,
        }
    }
}

impl<S: AsyncWrite + Unpin> EncryptedStream<S> {
    // Drives the currently-buffered record to `inner`. Clears the buffer when fully written.
    fn poll_flush_write(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.write_pos < self.write_buf.len() {
            match Pin::new(&mut self.inner).poll_write(cx, &self.write_buf[self.write_pos..]) {
                Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Poll::Ready(Ok(n)) => self.write_pos += n,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        self.write_buf.clear();
        self.write_pos = 0;
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for EncryptedStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        // If a record from a previous (backpressured) call is still buffered, finish
        // flushing it and report the bytes it represented. Do this before touching `buf`
        // so the same bytes are never encrypted twice.
        if !this.write_buf.is_empty() {
            ready!(this.poll_flush_write(cx))?;
            let n = this.pending_take;
            this.pending_take = 0;
            return Poll::Ready(Ok(n));
        }

        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let take = buf.len().min(MAX_PLAINTEXT);
        let mut msg = vec![0u8; take + NOISE_TAG_LEN];
        let n = this
            .noise
            .write_message(&buf[..take], &mut msg)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("noise encrypt: {}", e)))?;
        let len = u16::try_from(n)
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "noise record too large"))?;
        this.write_buf.clear();
        this.write_buf.extend_from_slice(&len.to_be_bytes());
        this.write_buf.extend_from_slice(&msg[..n]);
        this.write_pos = 0;
        this.pending_take = take;

        match this.poll_flush_write(cx) {
            Poll::Ready(Ok(())) => {
                this.pending_take = 0;
                Poll::Ready(Ok(take))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_flush_write(cx))?;
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_flush_write(cx))?;
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for EncryptedStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            // Serve any already-decrypted plaintext first.
            if this.read_plain_pos < this.read_plain.len() {
                let avail = &this.read_plain[this.read_plain_pos..];
                let n = avail.len().min(out.remaining());
                out.put_slice(&avail[..n]);
                this.read_plain_pos += n;
                return Poll::Ready(Ok(()));
            }

            // Otherwise read toward completing the current frame phase.
            let want = (this.read_need - this.read_acc.len()).min(8192);
            let mut tmp = [0u8; 8192];
            let mut rb = ReadBuf::new(&mut tmp[..want]);
            match Pin::new(&mut this.inner).poll_read(cx, &mut rb) {
                Poll::Ready(Ok(())) => {
                    let filled = rb.filled();
                    if filled.is_empty() {
                        // EOF: leave `out` empty to signal it upstream.
                        return Poll::Ready(Ok(()));
                    }
                    this.read_acc.extend_from_slice(filled);
                    if this.read_acc.len() == this.read_need {
                        match this.read_state {
                            ReadState::Len => {
                                let len = u16::from_be_bytes([this.read_acc[0], this.read_acc[1]])
                                    as usize;
                                this.read_acc.clear();
                                if len == 0 {
                                    return Poll::Ready(Err(io::Error::new(
                                        io::ErrorKind::InvalidData,
                                        "zero-length noise record",
                                    )));
                                }
                                this.read_state = ReadState::Body;
                                this.read_need = len;
                            }
                            ReadState::Body => {
                                let mut plain = vec![0u8; this.read_need];
                                let n = this
                                    .noise
                                    .read_message(&this.read_acc, &mut plain)
                                    .map_err(|e| {
                                        io::Error::new(
                                            io::ErrorKind::InvalidData,
                                            format!("noise decrypt: {}", e),
                                        )
                                    })?;
                                plain.truncate(n);
                                this.read_plain = plain;
                                this.read_plain_pos = 0;
                                this.read_acc.clear();
                                this.read_state = ReadState::Len;
                                this.read_need = 2;
                            }
                        }
                    }
                    // loop: serve freshly-decrypted plaintext or read the next chunk
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    fn to_hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{:02x}", x)).collect()
    }

    // Spec conformance: drive `snow` with the official Noise test vector for
    // Noise_NNpsk0_25519_ChaChaPoly_SHA256 (from haskell-cryptography/cacophony,
    // vectors/cacophony.txt) and assert the handshake message ciphertexts, the handshake
    // hash, and the first transport record match byte-for-byte. This is what actually
    // proves snow (and our parameters) implement the Noise spec, and therefore that the
    // Swift and Kotlin ports — which follow the same spec — will interoperate.
    #[test]
    fn official_noise_test_vector() {
        let prologue = hex("4a6f686e2047616c74");
        let psk: [u8; 32] = hex("54686973206973206d7920417573747269616e20706572737065637469766521")
            .try_into()
            .unwrap();
        let init_eph = hex("893e28b9dc6ca8d611ab664754b8ceb7bac5117349a4439a6b0569da977c464a");
        let resp_eph = hex("bbdb4cdbd309f1a1f2e1456967fe288cadd6f712d65dc7b7793d5e63da6b375b");

        let params: snow::params::NoiseParams = NOISE_PARAMS.parse().unwrap();
        let mut initiator = snow::Builder::new(params.clone())
            .fixed_ephemeral_key_for_testing_only(&init_eph)
            .prologue(&prologue)
            .unwrap()
            .psk(0, &psk)
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut responder = snow::Builder::new(params)
            .fixed_ephemeral_key_for_testing_only(&resp_eph)
            .prologue(&prologue)
            .unwrap()
            .psk(0, &psk)
            .unwrap()
            .build_responder()
            .unwrap();

        let mut buf = vec![0u8; MAX_NOISE_MESSAGE];
        let mut out = vec![0u8; MAX_NOISE_MESSAGE];

        // message 0: initiator -> responder
        let p0 = hex("4c756477696720766f6e204d69736573");
        let n0 = initiator.write_message(&p0, &mut buf).unwrap();
        assert_eq!(
            to_hex(&buf[..n0]),
            "ca35def5ae56cec33dc2036731ab14896bc4c75dbb07a61f879f8e3afa4c794479b962b8aff8485742ac32f905ba45369e2465fb59e138a93d67a0d1266b6a54"
        );
        let m0 = responder.read_message(&buf[..n0], &mut out).unwrap();
        assert_eq!(&out[..m0], &p0[..]);

        // message 1: responder -> initiator
        let p1 = hex("4d757272617920526f746862617264");
        let n1 = responder.write_message(&p1, &mut buf).unwrap();
        assert_eq!(
            to_hex(&buf[..n1]),
            "95ebc60d2b1fa672c1f46a8aa265ef51bfe38e7ccb39ec5be34069f144808843d6062704d5a9c422a8e834423f8c1feada7e8d0d910a1a2cd030fb584221e3"
        );
        let m1 = initiator.read_message(&buf[..n1], &mut out).unwrap();
        assert_eq!(&out[..m1], &p1[..]);

        // handshake hash matches the spec value
        let expected_hh = "f4d03dc34495c95729ea6de9e1b59004b59733102488b3e24bc441e0be208eaf";
        assert_eq!(to_hex(initiator.get_handshake_hash()), expected_hh);
        assert_eq!(to_hex(responder.get_handshake_hash()), expected_hh);

        // first transport message: initiator -> responder
        let mut it = initiator.into_transport_mode().unwrap();
        let mut rt = responder.into_transport_mode().unwrap();
        let p2 = hex("462e20412e20486179656b");
        let n2 = it.write_message(&p2, &mut buf).unwrap();
        assert_eq!(
            to_hex(&buf[..n2]),
            "e632c3763d7669067383433197a3baddf146e9e70ad4b4e9e59e0f"
        );
        let m2 = rt.read_message(&buf[..n2], &mut out).unwrap();
        assert_eq!(&out[..m2], &p2[..]);
    }

    #[test]
    fn psk_is_deterministic_and_32_bytes() {
        let a = derive_psk("hunter2hunter2");
        let b = derive_psk("hunter2hunter2");
        let c = derive_psk("different");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 32);
    }

    // Cross-platform known-answer test for the PSK derivation. Swift and Kotlin must
    // produce this exact value for password "flyingcarpet" (see docs §9).
    #[test]
    fn psk_known_answer() {
        let psk = derive_psk("flyingcarpet");
        let hex: String = psk.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(hex, PSK_KAT_HEX);
    }

    // A full NNpsk0 handshake + a transport message, with fixed ephemerals and a fixed PSK,
    // so any platform can reproduce the exact wire bytes (see docs §9).
    #[test]
    fn handshake_known_answer() {
        let psk = [0x2au8; 32];
        let init_eph = [0x01u8; 32];
        let resp_eph = [0x02u8; 32];
        let params: snow::params::NoiseParams = NOISE_PARAMS.parse().unwrap();

        let mut initiator = snow::Builder::new(params.clone())
            .fixed_ephemeral_key_for_testing_only(&init_eph)
            .psk(0, &psk)
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut responder = snow::Builder::new(params)
            .fixed_ephemeral_key_for_testing_only(&resp_eph)
            .psk(0, &psk)
            .unwrap()
            .build_responder()
            .unwrap();

        let mut buf = vec![0u8; MAX_NOISE_MESSAGE];

        let n1 = initiator.write_message(&[], &mut buf).unwrap();
        let msg1_hex: String = buf[..n1].iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(msg1_hex, HANDSHAKE_MSG1_HEX);

        let mut tmp = vec![0u8; MAX_NOISE_MESSAGE];
        responder.read_message(&buf[..n1], &mut tmp).unwrap();

        let n2 = responder.write_message(&[], &mut buf).unwrap();
        let msg2_hex: String = buf[..n2].iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(msg2_hex, HANDSHAKE_MSG2_HEX);

        initiator.read_message(&buf[..n2], &mut tmp).unwrap();

        let mut it = initiator.into_transport_mode().unwrap();
        let mut rt = responder.into_transport_mode().unwrap();

        let n3 = it.write_message(b"hello flying carpet", &mut buf).unwrap();
        let rec_hex: String = buf[..n3].iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(rec_hex, TRANSPORT_RECORD_HEX);

        let mut out = vec![0u8; MAX_NOISE_MESSAGE];
        let n = rt.read_message(&buf[..n3], &mut out).unwrap();
        assert_eq!(&out[..n], b"hello flying carpet");
    }

    // A realistic preamble transcript, as used by the app: version + mode, 8-byte BE each.
    fn test_prologue() -> Vec<u8> {
        let init_transcript = hex("000000000000000a0000000000000001"); // version 10, mode send
        let resp_transcript = hex("000000000000000a0000000000000000"); // version 10, mode receive
        build_prologue(&init_transcript, &resp_transcript)
    }

    async fn connect_pair(
        password_a: &str,
        password_b: &str,
    ) -> (
        Result<EncryptedStream<tokio::io::DuplexStream>, FCError>,
        Result<EncryptedStream<tokio::io::DuplexStream>, FCError>,
    ) {
        let (a, b) = tokio::io::duplex(64 * 1024);
        let pa = password_a.to_string();
        let pb = password_b.to_string();
        let ta =
            tokio::spawn(async move { handshake(a, Role::Initiator, &pa, &test_prologue()).await });
        let tb =
            tokio::spawn(async move { handshake(b, Role::Responder, &pb, &test_prologue()).await });
        (ta.await.unwrap(), tb.await.unwrap())
    }

    #[tokio::test]
    async fn round_trip_small_and_large() {
        let (ea, eb) = connect_pair("correct horse", "correct horse").await;
        let mut sender = ea.unwrap();
        let mut receiver = eb.unwrap();

        // small message
        sender
            .write_all(b"metadata: file.txt / 42 bytes")
            .await
            .unwrap();
        sender.flush().await.unwrap();
        let mut buf = [0u8; 29];
        receiver.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"metadata: file.txt / 42 bytes");

        // large message spanning many Noise records (> MAX_PLAINTEXT)
        let big: Vec<u8> = (0..500_000u32).map(|i| (i % 251) as u8).collect();
        let big2 = big.clone();
        let writer = tokio::spawn(async move {
            sender.write_all(&big2).await.unwrap();
            sender.flush().await.unwrap();
        });
        let mut got = vec![0u8; big.len()];
        receiver.read_exact(&mut got).await.unwrap();
        writer.await.unwrap();
        assert_eq!(got, big);
    }

    #[tokio::test]
    async fn wrong_password_fails_handshake() {
        let (ea, eb) = connect_pair("password-one", "password-two").await;
        // At least one side must fail to establish (the responder rejects msg1's tag).
        assert!(ea.is_err() || eb.is_err());
    }

    // Corrupting any single byte of a transport record must make the receiver's decrypt
    // fail (the Poly1305 tag no longer verifies). Drives the snow transport states
    // directly so the tampering happens on the actual record bytes.
    #[test]
    fn tampering_is_detected() {
        let psk = derive_psk("shared secret");
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
        let mut buf = vec![0u8; MAX_NOISE_MESSAGE];
        let mut tmp = vec![0u8; MAX_NOISE_MESSAGE];
        let n = initiator.write_message(&[], &mut buf).unwrap();
        responder.read_message(&buf[..n], &mut tmp).unwrap();
        let n = responder.write_message(&[], &mut buf).unwrap();
        initiator.read_message(&buf[..n], &mut tmp).unwrap();
        let mut it = initiator.into_transport_mode().unwrap();
        let mut rt = responder.into_transport_mode().unwrap();

        // first record, untampered: decrypts fine (sanity-checks the setup)
        let n = it.write_message(b"authentic payload", &mut buf).unwrap();
        let m = rt.read_message(&buf[..n], &mut tmp).unwrap();
        assert_eq!(&tmp[..m], b"authentic payload");

        // second record with one bit flipped in the ciphertext: must fail to authenticate
        let n = it.write_message(b"authentic payload", &mut buf).unwrap();
        buf[n / 2] ^= 0x01;
        assert!(
            rt.read_message(&buf[..n], &mut tmp).is_err(),
            "tampered record must fail authentication"
        );
    }

    // Same guarantee for a corrupted handshake message: the responder must reject it.
    #[test]
    fn tampered_handshake_is_detected() {
        let psk = derive_psk("shared secret");
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
        let mut buf = vec![0u8; MAX_NOISE_MESSAGE];
        let mut tmp = vec![0u8; MAX_NOISE_MESSAGE];
        let n = initiator.write_message(&[], &mut buf).unwrap();
        // corrupt the encrypted payload section (past the 32-byte ephemeral)
        buf[n - 1] ^= 0x01;
        assert!(
            responder.read_message(&buf[..n], &mut tmp).is_err(),
            "tampered handshake message must fail authentication"
        );
    }

    // The prologue binds the plaintext preamble: if the two sides saw different preamble
    // bytes (an in-path attacker rewrote the version or mode exchange), the handshake
    // must fail even though the passwords match.
    #[tokio::test]
    async fn prologue_mismatch_fails_handshake() {
        let (a, b) = tokio::io::duplex(64 * 1024);
        let good = test_prologue();
        // attacker flipped the mode bit the responder saw
        let mut tampered_resp_transcript = hex("000000000000000a0000000000000000");
        tampered_resp_transcript[15] ^= 0x01;
        let tampered = build_prologue(
            &hex("000000000000000a0000000000000001"),
            &tampered_resp_transcript,
        );
        let ta = tokio::spawn(async move { handshake(a, Role::Initiator, "pw", &good).await });
        let tb = tokio::spawn(async move { handshake(b, Role::Responder, "pw", &tampered).await });
        let (ea, eb) = (ta.await.unwrap(), tb.await.unwrap());
        assert!(ea.is_err() || eb.is_err());
    }

    // Cross-platform known-answer for the prologue-bound handshake (docs §9): the exact
    // wire bytes any platform must produce for the fixed app-style preamble transcript,
    // PSK, and ephemerals. Also pins build_prologue's framing (u64-BE length prefixes).
    #[test]
    fn prologue_known_answer() {
        let init_transcript = hex("000000000000000a0000000000000001");
        let resp_transcript = hex("000000000000000a0000000000000000");
        let prologue = build_prologue(&init_transcript, &resp_transcript);
        assert_eq!(to_hex(&prologue), PROLOGUE_KAT_HEX);

        let psk = [0x2au8; 32];
        let init_eph = [0x01u8; 32];
        let resp_eph = [0x02u8; 32];
        let params: snow::params::NoiseParams = NOISE_PARAMS.parse().unwrap();
        let mut initiator = snow::Builder::new(params.clone())
            .fixed_ephemeral_key_for_testing_only(&init_eph)
            .prologue(&prologue)
            .unwrap()
            .psk(0, &psk)
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut responder = snow::Builder::new(params)
            .fixed_ephemeral_key_for_testing_only(&resp_eph)
            .prologue(&prologue)
            .unwrap()
            .psk(0, &psk)
            .unwrap()
            .build_responder()
            .unwrap();

        let mut buf = vec![0u8; MAX_NOISE_MESSAGE];
        let mut tmp = vec![0u8; MAX_NOISE_MESSAGE];

        let n1 = initiator.write_message(&[], &mut buf).unwrap();
        assert_eq!(to_hex(&buf[..n1]), PROLOGUE_MSG1_HEX);
        responder.read_message(&buf[..n1], &mut tmp).unwrap();

        let n2 = responder.write_message(&[], &mut buf).unwrap();
        assert_eq!(to_hex(&buf[..n2]), PROLOGUE_MSG2_HEX);
        initiator.read_message(&buf[..n2], &mut tmp).unwrap();

        let mut it = initiator.into_transport_mode().unwrap();
        let mut rt = responder.into_transport_mode().unwrap();
        let n3 = it.write_message(b"hello flying carpet", &mut buf).unwrap();
        assert_eq!(to_hex(&buf[..n3]), PROLOGUE_RECORD_HEX);
        let n = rt.read_message(&buf[..n3], &mut tmp).unwrap();
        assert_eq!(&tmp[..n], b"hello flying carpet");
    }
}

// Filled in from the test output (see docs §9). Reproduced by every platform.
#[cfg(test)]
const PSK_KAT_HEX: &str = "a3d8b7f17f2252e4c2847a365ab2f392beaa996b7e51dd6fa19ff1ad08938619";
#[cfg(test)]
const HANDSHAKE_MSG1_HEX: &str =
    "a4e09292b651c278b9772c569f5fa9bb13d906b46ab68c9df9dc2b4409f8a209a3e9c18456aba2185de800ffaca55b22";
#[cfg(test)]
const HANDSHAKE_MSG2_HEX: &str =
    "ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59d887595caf8a0b110dfab84e6b41eafc";
#[cfg(test)]
const TRANSPORT_RECORD_HEX: &str =
    "124a00c03b4544f746828bbf9ae2d8d595a9ac1fea988f43f7206c3880180b954f9147";
// Prologue-bound handshake KAT (v10 preamble transcript; see prologue_known_answer).
#[cfg(test)]
const PROLOGUE_KAT_HEX: &str = "0000000000000010000000000000000a00000000000000010000000000000010000000000000000a0000000000000000";
#[cfg(test)]
const PROLOGUE_MSG1_HEX: &str =
    "a4e09292b651c278b9772c569f5fa9bb13d906b46ab68c9df9dc2b4409f8a2093ae03dc8524f79ac9696d6c155df9a3c";
#[cfg(test)]
const PROLOGUE_MSG2_HEX: &str =
    "ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59d2668070263116ce557500fbe3fd3ba4";
// Identical to TRANSPORT_RECORD_HEX by design: the prologue only enters `h` (so it gates
// the handshake message MACs — see PROLOGUE_MSG1/2 differing from HANDSHAKE_MSG1/2), never
// the chaining key, so Split() derives the same transport keys. Asserted anyway so a port
// that wrongly mixes the prologue into ck fails here.
#[cfg(test)]
const PROLOGUE_RECORD_HEX: &str =
    "124a00c03b4544f746828bbf9ae2d8d595a9ac1fea988f43f7206c3880180b954f9147";
