#[cfg_attr(target_os = "linux", path = "linux/network.rs")]
#[cfg_attr(target_os = "windows", path = "windows/network.rs")]
pub mod network;

#[cfg_attr(target_os = "linux", path = "linux/bluetooth.rs")]
#[cfg_attr(target_os = "windows", path = "windows/bluetooth.rs")]
pub mod bluetooth;

pub mod discovery;
pub mod error;
pub mod noise;
mod receiving;
mod sending;
pub mod utils;

use bluetooth::negotiate_bluetooth;
use discovery::{DiscoveryRole, DiscoveryService};
use error::{fc_error, FCError};
use std::{
    net::SocketAddr,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};
use utils::get_key_and_ssid;

/// The transport the transfer runs over: a plain TCP stream (hotspot mode) or a Noise
/// EncryptedStream (shared network mode, v10+). Both implement AsyncRead + AsyncWrite, so
/// version/mode confirmation and the send/receive code are identical over either. Opaque
/// to callers — `start_transfer` returns it and `clean_up_transfer` consumes it.
pub enum TransferStream {
    Plain(TcpStream),
    Encrypted(Box<noise::EncryptedStream<TcpStream>>),
}

impl AsyncRead for TransferStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransferStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            TransferStream::Encrypted(s) => Pin::new(&mut **s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for TransferStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            TransferStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            TransferStream::Encrypted(s) => Pin::new(&mut **s).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransferStream::Plain(s) => Pin::new(s).poll_flush(cx),
            TransferStream::Encrypted(s) => Pin::new(&mut **s).poll_flush(cx),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransferStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            TransferStream::Encrypted(s) => Pin::new(&mut **s).poll_shutdown(cx),
        }
    }
}

const CHUNKSIZE: usize = 1_000_000; // 1 MB
                                    // v10 is a breaking change: shared network mode and its new protocol are not compatible
                                    // with v9 or earlier. See docs/shared-network-crypto.md.
const MAJOR_VERSION: u64 = 10;
// Sanity bound on the peer-supplied file count (companion to the header bounds in
// receiving.rs): no legitimate transfer approaches it, and a corrupt or hostile
// stream shouldn't be able to put us into a near-endless receive loop.
const MAX_FILE_COUNT: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConnectionMode {
    Hotspot,
    SharedNetwork,
}

pub trait UI: Clone + Send + 'static {
    fn output(&self, msg: &str);
    fn show_progress_bar(&self);
    fn update_progress_bar(&self, percent: u8);
    fn enable_ui(&self);
    fn show_pin(&self, pin: &str);
}

#[derive(Clone)]
pub enum Mode {
    Send(Vec<SendFile>),
    Receive(PathBuf),
}

// A file queued for sending, paired with the relative name the peer will receive it under.
//
// The name is computed once at selection time (utils::expand_selection) instead of being
// derived from a common prefix at send time: every top-level selection is stripped of its
// own parent directory, so a selected folder's name survives on the wire and the receiver
// recreates the folder with the files inside, while individually selected files arrive
// flat. Separators are always "/", whatever the host platform uses. Matches the Apple and
// Android senders; see docs/send-folder-behavior.md.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct SendFile {
    pub path: PathBuf,
    pub name: String,
}

#[derive(Clone, Copy)]
pub enum Peer {
    Android,
    IOS,
    Linux,
    MacOS,
    Windows,
}

impl TryFrom<&str> for Peer {
    type Error = FCError;

    fn try_from(peer: &str) -> Result<Self, Self::Error> {
        match peer {
            "android" => Ok(Peer::Android),
            "ios" => Ok(Peer::IOS),
            "linux" => Ok(Peer::Linux),
            "mac" => Ok(Peer::MacOS),
            "windows" => Ok(Peer::Windows),
            other => Err(FCError {
                message: format!("Bad peer: {}", other),
            }),
        }
    }
}

pub enum PeerResource {
    WifiClient(String), // used if joining, .0 is ip of gateway/peer/host
    WindowsHotspot(network::WindowsHotspot),
    LinuxHotspot,
}

// first String is the interface's name, second String is a base-10 representation of the u128 representation of the GUID of the interface. GUID is only used on Windows.
#[derive(serde::Deserialize, serde::Serialize)]
pub struct WiFiInterface(pub String, pub String);

// returned by the interface-enumeration functions so the UI can label interfaces with
// their IP (or lack of one); name and guid follow the WiFiInterface conventions above
#[derive(serde::Serialize)]
pub struct InterfaceInfo {
    pub name: String,
    pub guid: String,
    pub ip: Option<String>,
}

pub struct Transfer {
    pub cancel_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub hotspot: Arc<Mutex<Option<PeerResource>>>,
    pub ssid: Arc<Mutex<Option<String>>>,
    pub ble_ui_tx: Mutex<Option<mpsc::Sender<bool>>>, // used by javascript to report user's choice about whether to pair with bluetooth device to windows custom pairing callback.
}

impl Transfer {
    pub fn new() -> Self {
        Transfer {
            cancel_handle: Mutex::new(None),
            hotspot: Arc::new(Mutex::new(None)),
            ssid: Arc::new(Mutex::new(None)),
            ble_ui_tx: Mutex::new(None),
        }
    }
}

pub async fn start_transfer<T: UI>(
    mode: String,
    using_bluetooth: bool,
    mut peer: Option<String>,
    mut password: Option<String>,
    interface: WiFiInterface,
    file_list: Option<Vec<SendFile>>,
    receive_dir: Option<String>,
    ui: &T,
    hotspot: Arc<Mutex<Option<PeerResource>>>,
    state_ssid: Arc<Mutex<Option<String>>>,
    ble_ui_rx: mpsc::Receiver<bool>,
    connection_mode: ConnectionMode,
) -> Option<TransferStream> {
    // get files or receive directory
    // don't panic on bad input: a panic here kills the transfer task without running
    // cleanup, which is how the UI used to get stuck in its in-progress state (#118)
    let mode = if mode == "send" {
        // an empty list is treated as "nothing chosen" rather than started: javascript's
        // truthiness check passes an empty array through, and a zero-file send has nothing
        // to do anyway
        let files = match file_list {
            Some(files) if !files.is_empty() => files,
            _ => {
                ui.output("Error: send mode selected but no files were chosen.");
                return None;
            }
        };
        Mode::Send(files)
    } else if mode == "receive" {
        match receive_dir {
            Some(folder) => Mode::Receive(PathBuf::from(folder)),
            None => {
                ui.output("Error: receive mode selected but no destination folder was chosen.");
                return None;
            }
        }
    } else {
        ui.output(&format!("Error: bad mode: {}", mode));
        return None;
    };

    // if bluetooth, make that connection here first
    // for windows and linux, the central/client api can read and write synchronously, and we always know the ssid before starting hotspot, so we can just do that here before connecting to peer?
    // for servers/peripherals, does it matter? callbacks in both cases?

    // Bluetooth is hotspot-only: in shared network mode the password is exchanged manually
    // (receiver displays it, sender types it) and discovery finds the peer over IP.
    let using_bluetooth = using_bluetooth && connection_mode == ConnectionMode::Hotspot;

    if using_bluetooth {
        match negotiate_bluetooth(&mode, ble_ui_rx, ui).await {
            Ok((p, _ssid, pw)) => {
                peer = Some(p);
                if password.is_none() {
                    password = Some(pw);
                }
            }
            Err(e) => {
                ui.output(&format!("Could not establish Bluetooth connection: {}", e));
                println!("Could not establish Bluetooth connection: {}", e);
                return None;
            }
        }
    }

    let password = match password {
        Some(p) => p,
        None => {
            ui.output("Error: no password provided for transfer.");
            return None;
        }
    };
    let (_, ssid) = get_key_and_ssid(&password);

    // Derive the Noise PSK once, up front: the handshake needs it, and in shared network
    // mode the discovery HMAC key is derived from it too (see noise::derive_discovery_key)
    // so that no fast hash of the password ever goes on the air. Same PBKDF2 cost as
    // before — it previously ran inside the handshake — just moved before discovery.
    let psk = noise::derive_psk(&password);

    {
        let mut _state_ssid = state_ssid.lock().expect("Couldn't lock state_ssid");
        *_state_ssid = Some(ssid.clone());
    }

    // Establish the raw TCP connection and determine the Noise role. The Noise initiator
    // must be the TCP client (it sends the first handshake message). No handshake yet:
    // version and mode are negotiated in plaintext first (below), then the connection is
    // wrapped in Noise, for BOTH modes.
    let (peer_resource, tcp, noise_role) = match connection_mode {
        ConnectionMode::SharedNetwork => {
            // Shared Network Mode: Use discovery to find peer on existing network. Both
            // sides are labeled WifiClient, so the role comes from send/receive: the sender
            // is the TCP client (initiator), the receiver is the TCP server (responder).
            match start_shared_network_transfer(
                &mode,
                &noise::derive_discovery_key(&psk),
                &interface,
                ui,
            )
            .await
            {
                Ok((resource, tcp)) => {
                    let role = if matches!(mode, Mode::Send(_)) {
                        noise::Role::Initiator
                    } else {
                        noise::Role::Responder
                    };
                    (resource, tcp, role)
                }
                Err(e) => {
                    ui.output(&format!("Error in shared network mode: {}", e));
                    return None;
                }
            }
        }
        ConnectionMode::Hotspot => {
            // Original Hotspot Mode
            let peer = match Peer::try_from(
                peer.expect("Neither UI nor Bluetooth peer present.")
                    .as_str(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    ui.output(&format!("Error parsing peer: {}", e));
                    return None;
                }
            };

            // start hotspot or connect to peer's (the Noise handshake below uses the
            // already-derived PSK, not the password itself)
            let peer_resource = match network::connect_to_peer(
                peer,
                mode.clone(),
                ssid,
                password,
                interface,
                ui,
            )
            .await
            {
                Ok(p) => p,
                Err(e) => {
                    ui.output(&format!("Error connecting to peer: {}", e));
                    return None;
                }
            };

            tokio::task::yield_now().await;

            // start tcp connection
            let stream = match start_tcp(&peer_resource, ui).await {
                Ok(s) => s,
                Err(e) => {
                    ui.output(&format!("Error starting TCP connection: {}", e));
                    return None;
                }
            };

            // The hotspot host is the TCP server (responder); the guest that joined and
            // connected is the client (initiator). start_tcp uses exactly this split.
            let role = match peer_resource {
                PeerResource::WifiClient(_) => noise::Role::Initiator,
                _ => noise::Role::Responder,
            };
            (peer_resource, stream, role)
        }
    };

    // The confirm functions only need to know whether we joined the peer's network (guest
    // sends first) or are hosting; capture that before peer_resource moves into the state.
    let is_wifi_client = matches!(peer_resource, PeerResource::WifiClient(..));

    // Store the hotspot in tauri's state NOW, before anything below can fail, so that
    // clean_up_transfer tears it down even when the preamble or handshake errors out
    // (on Windows, stop_hotspot is a no-op unless the PeerResource is in this state).
    // Has to be in its own block or tokio complains that this "mutex guard" is held across an await.
    {
        let mut hotspot_value = hotspot.lock().expect("Couldn't lock hotspot mutex");
        *hotspot_value = Some(peer_resource);
    }

    // Plaintext preamble on the raw TCP stream: version, then send/receive mode. These are
    // not secret (an eavesdropper can already see a transfer is happening) and keeping them
    // outside Noise gives clean version-mismatch reporting — but every preamble byte, sent
    // and received, is recorded and bound into the Noise prologue below, so tampering with
    // them fails the handshake instead of going unnoticed.
    let mut preamble = noise::RecordingStream::new(tcp);

    // make sure the versions are compatible
    match confirm_version(is_wifi_client, &mut preamble).await {
        Ok(()) => (),
        Err(e) => {
            ui.output(&format!("Error confirming version: {}", e));
            let (tcp, _, _) = preamble.into_parts();
            return Some(TransferStream::Plain(tcp));
        }
    };

    // confirm that one end is sending and the other is receiving
    match confirm_mode(mode.clone(), is_wifi_client, &mut preamble, connection_mode).await {
        Ok(()) => (),
        Err(e) => {
            ui.output(&format!("Error confirming mode: {}", e));
            let (tcp, _, _) = preamble.into_parts();
            return Some(TransferStream::Plain(tcp));
        }
    };

    // Now establish the Noise encrypted transport over the same connection, for both modes,
    // with the preamble transcript bound in as the prologue. Everything after this — file
    // count, metadata, and file data — is confidential and tamper-evident. A wrong password
    // (or a tampered preamble) fails the handshake with a clear message.
    let (tcp, sent, received) = preamble.into_parts();
    let prologue = match noise_role {
        noise::Role::Initiator => noise::build_prologue(&sent, &received),
        noise::Role::Responder => noise::build_prologue(&received, &sent),
    };
    ui.output("Establishing encrypted connection...");
    let mut stream = match noise::handshake(tcp, noise_role, &psk, &prologue).await {
        Ok(enc) => {
            ui.output("Encrypted connection established.");
            TransferStream::Encrypted(Box::new(enc))
        }
        Err(e) => {
            ui.output(&format!("{}", e));
            return None;
        }
    };

    match mode {
        Mode::Send(files) => {
            // tell receiving end how many files we're sending
            match stream.write_u64(files.len() as u64).await {
                Ok(()) => (),
                Err(e) => {
                    ui.output(&format!("Error writing number of files: {}", e));
                    return Some(stream);
                }
            }
            // send files. each file already carries the relative name the peer will store
            // it under, resolved at selection time by utils::expand_selection
            for (i, file) in files.iter().enumerate() {
                ui.output("=========================");
                ui.output(&format!(
                    "Sending file {} of {}. Filename: {}",
                    i + 1,
                    files.len(),
                    file.name
                ));
                match sending::send_file(&file.path, &file.name, &mut stream, ui).await {
                    Ok(_) => (),
                    Err(e) => {
                        ui.output(&format!("Error sending file: {}", e));
                        return Some(stream);
                    }
                };
            }
        }
        Mode::Receive(folder) => {
            // find out how many files we're receiving
            let num_files = match stream.read_u64().await {
                Ok(num) => num,
                Err(e) => {
                    ui.output(&format!("Error reading number of files: {}", e));
                    return Some(stream);
                }
            };
            if num_files > MAX_FILE_COUNT {
                ui.output(&format!(
                    "Error: file count {} from peer is out of range",
                    num_files
                ));
                return Some(stream);
            }
            // receive files
            for i in 0..num_files {
                ui.output("=========================");
                ui.output(&format!("Receiving file {} of {}.", i + 1, num_files,));
                let last_file = i == num_files - 1;
                match receiving::receive_file(&folder, &mut stream, ui, last_file).await {
                    Ok(_) => (),
                    Err(e) => {
                        ui.output(&format!("Error receiving file: {}", e));
                        return Some(stream);
                    }
                }
            }
        }
    }

    ui.output("=========================");
    ui.output("Transfer complete");
    Some(stream)
}

pub async fn clean_up_transfer<T: UI>(
    stream: Option<TransferStream>,
    hotspot: Arc<Mutex<Option<PeerResource>>>,
    ssid: Arc<Mutex<Option<String>>>,
    ui: &T,
) {
    // shut down tcp stream
    match stream {
        Some(mut s) => {
            if s.shutdown().await.is_err() {
                ui.output("Failed to shut down TCP stream.")
            };
        }
        None => (),
    }
    // shut down hotspot
    shut_down_hotspot(&hotspot, &ssid, ui);
    // make sure hotspot gets dropped
    let mut hotspot_value = hotspot.lock().expect("Couldn't lock hotspot mutex");
    *hotspot_value = None;
    // enable UI
    ui.enable_ui();
}

fn shut_down_hotspot<T: UI>(
    hotspot: &Arc<Mutex<Option<PeerResource>>>,
    ssid: &Arc<Mutex<Option<String>>>,
    _ui: &T,
) {
    let peer_resource = hotspot.lock().expect("Couldn't lock hotspot mutex.");
    let peer_resource = peer_resource.as_ref();
    let ssid = ssid.lock().expect("Couldn't lock SSID mutex.");
    match network::stop_hotspot(peer_resource, ssid.as_deref()) {
        Err(e) => println!("{}", e),
        Ok(msg) => println!("{}", msg),
    };
}

async fn start_tcp<T: UI>(peer_resource: &PeerResource, ui: &T) -> Result<TcpStream, FCError> {
    let stream;
    match peer_resource {
        PeerResource::WifiClient(gateway) => {
            let addr = format!("{}:3290", gateway).parse::<SocketAddr>()?;
            stream = TcpStream::connect(addr).await?;
        }
        _ => {
            // linux or windows hotspot
            let addr = "0.0.0.0:3290".parse::<SocketAddr>()?;
            let listener = TcpListener::bind(&addr).await?;
            ui.output("Waiting for connection...");
            let (_stream, _socket_addr) = listener.accept().await?;
            ui.output("Connection accepted");
            stream = _stream;
        }
    }
    Ok(stream)
}

async fn start_shared_network_transfer<T: UI>(
    mode: &Mode,
    discovery_key: &[u8; 32],
    interface: &WiFiInterface,
    ui: &T,
) -> Result<(PeerResource, TcpStream), FCError> {
    // Check for network connection
    if !network::has_network_connection(interface)? {
        fc_error("No network connection on selected interface")?;
    }

    // Both roles need inbound traffic on port 3290: UDP for discovery announcements,
    // and TCP for the receiver's listener. No-op on Linux.
    network::ensure_firewall_rules(ui).await?;

    // Get local IP and prefix length
    let local_ip = network::get_local_ip(interface)?;
    let prefix_len = network::get_prefix_length(interface)?;
    ui.output(&format!("Local IP: {}/{}", local_ip, prefix_len));

    // Determine role for TCP connection
    let role = DiscoveryRole::from(mode);

    // Receiver is TCP server (consistent with hotspot same-platform convention
    // where the receiver hosts). Bind listener *before* discovery so it's ready
    // when the sender connects immediately after discovering us.
    let listener = if role == DiscoveryRole::Receiver {
        let addr = "0.0.0.0:3290".parse::<SocketAddr>()?;
        let listener = TcpListener::bind(&addr).await?;
        ui.output("TCP listener ready on port 3290.");
        Some(listener)
    } else {
        None
    };

    // Create discovery service
    let discovery = DiscoveryService::new(*discovery_key, mode, local_ip, prefix_len);

    let (peer_ip, stream) = match role {
        DiscoveryRole::Receiver => {
            // The sender discovers us and connects, and it stops announcing as soon as
            // it hears us — possibly before we ever hear it. So the TCP connection
            // itself is the receiver's completion signal: discovery runs alongside the
            // listener only to announce our presence and surface diagnostics
            // (receiver-role discovery never resolves with a peer), and must not gate
            // the accept. No timeout on the accept either: the sender may not be
            // started for a long time.
            let listener = listener.unwrap();
            tokio::select! {
                result = discovery.discover_peer(ui) => {
                    // only returns on failure or cancellation
                    result?;
                    fc_error("Discovery ended unexpectedly")?;
                    unreachable!()
                }
                accepted = listener.accept() => {
                    let (stream, addr) = accepted?;
                    ui.output(&format!("TCP connection accepted from {}", addr));
                    (addr.ip().to_string(), stream)
                }
            }
        }
        DiscoveryRole::Sender => {
            let peer_ip = discovery.discover_peer(ui).await?;

            // Sender connects to receiver
            ui.output(&format!("Connecting to peer at {}:3290", peer_ip));

            // Retry for up to 30 seconds (matches the Apple implementation): the
            // receiver may still be finishing discovery when we start connecting.
            const CONNECT_ATTEMPTS: u32 = 15;
            let mut stream = None;
            for attempt in 1..=CONNECT_ATTEMPTS {
                match TcpStream::connect(format!("{}:3290", peer_ip)).await {
                    Ok(s) => {
                        stream = Some(s);
                        break;
                    }
                    Err(e) => {
                        if attempt < CONNECT_ATTEMPTS {
                            ui.output(&format!(
                                "Connection attempt {} failed, retrying...",
                                attempt
                            ));
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        } else {
                            fc_error(&format!("Failed to connect to peer: {}", e))?;
                        }
                    }
                }
            }

            (peer_ip.to_string(), stream.unwrap())
        }
    };

    ui.output("TCP connection established");
    Ok((PeerResource::WifiClient(peer_ip), stream))
}

async fn confirm_mode<S: AsyncRead + AsyncWrite + Unpin>(
    mode: Mode,
    is_wifi_client: bool,
    stream: &mut S,
    connection_mode: ConnectionMode,
) -> Result<(), FCError> {
    let our_mode: u64 = match mode {
        Mode::Send(..) => 1,
        Mode::Receive(..) => 0,
    };

    match connection_mode {
        ConnectionMode::SharedNetwork => {
            // Symmetric approach (matches Apple implementation):
            // Both sides send their mode, both sides read peer's mode, both verify opposite
            stream.write_u64(our_mode).await?;
            let peer_mode = stream.read_u64().await?;
            if peer_mode == our_mode {
                let msg = format!(
                    "Both ends of the transfer selected {}",
                    if our_mode == 0 { "receive" } else { "send" }
                );
                fc_error(&msg)?
            }
        }
        ConnectionMode::Hotspot => {
            // Asymmetric approach for backward compatibility with hotspot mode
            if is_wifi_client {
                // tell host what mode we selected and wait for confirmation that they don't match
                stream.write_u64(our_mode).await?;
                // wait to ensure host responds that mode selection was correct
                if stream.read_u64().await? != 1 {
                    let message = format!(
                        "Both ends of the transfer selected {}",
                        if our_mode == 0 { "receive" } else { "send" }
                    );
                    fc_error(&message)?
                }
            } else {
                // hosting: wait for guest to say what mode they selected, compare to our own, and report back
                let peer_mode = stream.read_u64().await?;
                if peer_mode == our_mode {
                    let msg = format!(
                        "Both ends of the transfer selected {}",
                        if our_mode == 0 { "receive" } else { "send" }
                    );
                    // write failure to guest
                    stream.write_u64(0).await?;
                    fc_error(&msg)?
                } else {
                    // write success to guest
                    stream.write_u64(1).await?;
                }
            }
        }
    }
    Ok(())
}

async fn confirm_version<S: AsyncRead + AsyncWrite + Unpin>(
    is_wifi_client: bool,
    stream: &mut S,
) -> Result<(), FCError> {
    // only really have to worry about version 6 as that's the only one online and in app store. it will do mode confirmation first,
    // and obey hotspot host/guest rule, and it will write 0 or 1 for mode, so we shouldn't deadlock with both ends waiting.
    let peer_version = if is_wifi_client {
        // send version to hotspot host. in shared network mode both sides are wifi
        // clients, so both send first — symmetric, works via TCP buffering.
        stream.write_u64(MAJOR_VERSION).await?;
        // receive version of host
        stream.read_u64().await?
    } else {
        // wait for guest to say what version they're using, then send our version
        let _peer_version = stream.read_u64().await?;
        stream.write_u64(MAJOR_VERSION).await?;
        _peer_version
    };

    if peer_version < MAJOR_VERSION {
        // we make decision
        if utils::is_compatible(peer_version) {
            stream.write_u64(1).await?; // report that versions are compatible
        } else {
            stream.write_u64(0).await?;
            fc_error(&format!("The other device is running Flying Carpet version {}, which is not compatible with this version ({}). Please update both devices to the latest version at https://flyingcarpet.spiegl.dev.", peer_version, MAJOR_VERSION))?;
        }
    } else if peer_version > MAJOR_VERSION {
        // peer makes decision
        if stream.read_u64().await? == 0 {
            fc_error(&format!("The other device is running Flying Carpet version {}, which is not compatible with this version ({}). Please update both devices to the latest version at https://flyingcarpet.spiegl.dev.", peer_version, MAJOR_VERSION))?;
        }
    } // otherwise, versions match, implicitly compatible
    Ok(())
}

// TODO:
// drag and drop shouldn't work when already in transfer
// linux can't receive from windows or android if already paired/connected, service not found. but then it disconnects and next transfer works. unpair after every transfer?
// don't write ssid over bluetooth till hotspot has started, so that peer (especially iOS) doesn't start trying too early.
// test closing about window with x on linux: panic?
// https://github.com/hbldh/bleak/issues/367#issuecomment-784375835
// linux name is null on android when pairing - manufacturer info?
// fix bug where multiple start/cancel clicks stack while waiting for transfer to cancel, at least on linux: have to get whatever is blocking on background thread?
// show qr code after refresh

// TESTS:
// test multiple transfers back to back, windows central unpaired but ios peripheral still paired, already paired but switched mode
// test switching os...
// fix tests
// test pulling wifi card, quitting program, etc.

// MYSTERIES
// "Corrupt JPEG data: 298 extraneous bytes before marker 0xbb" in debug output on windows
// how did windows read OS "windows" from itself when acting as central but not peripheral? windows previously wrote "windows" to the OS characteristic of android, which stored it? doesn't look like it from the android code.
// linux sending to linux: last file sent but then hung, didn't exit transfer. receiving end said "didn't receive confirmation".
// is the problem that the device we see advertising isn't the device we're already paired to? but then the device we're paired to presumably offers the services already.

// LATER MAYBE:
// code signing for windows?
// faster?
// cli version?
// move expand_files into utils and make tauri's version a wrapper for CLI version
// hosted network stuff on windows?
// send folder mode?
// recreate directory structure if all submitted files are in same dir. taken for granted in gui? only problem for cli? not if dropping appends... only allow when using send-folder?
// remove file selection box and replace start button with Choose Files/Choose Folder? gets in the way of drag and drop... so no?
// optional password length?
// move password length constant into rust, fetch in javascript

#[cfg(test)]
mod transfer_tests {
    use super::*;
    use crate::noise::{handshake, Role};

    #[derive(Clone)]
    struct TestUi;
    impl UI for TestUi {
        fn output(&self, _msg: &str) {}
        fn show_progress_bar(&self) {}
        fn update_progress_bar(&self, _percent: u8) {}
        fn enable_ui(&self) {}
        fn show_pin(&self, _pin: &str) {}
    }

    // End-to-end shared-network path: the real send_file/receive_file run over a Noise
    // EncryptedStream (backed by an in-memory duplex), with a >64 KiB file so the transfer
    // spans multiple Noise records. Verifies handshake, encrypted metadata, chunk transfer,
    // and byte-exact file integrity through the whole stack.
    #[tokio::test]
    async fn end_to_end_encrypted_transfer() {
        let base = std::env::temp_dir().join(format!("fc_noise_test_{}", std::process::id()));
        let send_dir = base.join("send");
        let recv_dir = base.join("recv");
        std::fs::create_dir_all(&send_dir).unwrap();
        std::fs::create_dir_all(&recv_dir).unwrap();
        let src = send_dir.join("photo.bin");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
        std::fs::write(&src, &data).unwrap();

        let psk = noise::derive_psk("correct horse battery staple");
        // both sides bind the same preamble transcript, as the real flow does
        let prologue = noise::build_prologue(
            &[0, 0, 0, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 1],
            &[0, 0, 0, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        let prologue2 = prologue.clone();

        let (a, b) = tokio::io::duplex(64 * 1024);
        let src2 = src.clone();
        let recv_dir2 = recv_dir.clone();

        let sender = tokio::spawn(async move {
            let mut enc = handshake(a, Role::Initiator, &psk, &prologue)
                .await
                .unwrap();
            enc.write_u64(1).await.unwrap(); // file count, as the orchestrator does
            // sent under a folder-relative name, as a "send folder" selection produces,
            // so the receiver's directory recreation is covered end to end
            sending::send_file(&src2, "album/photo.bin", &mut enc, &TestUi)
                .await
                .unwrap();
            enc.flush().await.unwrap();
        });
        let receiver = tokio::spawn(async move {
            let mut enc = handshake(b, Role::Responder, &psk, &prologue2)
                .await
                .unwrap();
            let count = enc.read_u64().await.unwrap();
            assert_eq!(count, 1);
            receiving::receive_file(&recv_dir2, &mut enc, &TestUi, true)
                .await
                .unwrap();
        });
        sender.await.unwrap();
        receiver.await.unwrap();

        let got = std::fs::read(recv_dir.join("album").join("photo.bin")).unwrap();
        assert_eq!(
            got, data,
            "received file must match sent file byte-for-byte"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
