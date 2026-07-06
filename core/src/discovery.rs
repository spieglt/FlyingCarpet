use crate::error::{DiscoveryError, FCError};
use crate::utils::{compute_hmac, verify_hmac};
use crate::{Mode, UI};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::interval;

pub const MULTICAST_ADDR: &str = "239.255.73.67";
pub const DISCOVERY_PORT: u16 = 3290;
pub const DISCOVERY_MAGIC: [u8; 4] = *b"FCAP";
pub const ANNOUNCEMENT_SIZE: usize = 93;
const TIMESTAMP_WINDOW_SECS: u64 = 60;
const DISCOVERY_INTERVAL_MS: u64 = 500;
const MAX_UNICAST_SCAN_HOSTS: u32 = 1024;

// Compile-time check that ANNOUNCEMENT_SIZE matches the sum of all field sizes.
const _: () = assert!(
    4 + 2 + 1 + 4 + 4 + 2 + 8 + 4 + 32 + 32 == ANNOUNCEMENT_SIZE,
    "ANNOUNCEMENT_SIZE does not match sum of field sizes"
);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiscoveryRole {
    Sender = 0,
    Receiver = 1,
}

impl From<&Mode> for DiscoveryRole {
    fn from(mode: &Mode) -> Self {
        match mode {
            Mode::Send(_) => DiscoveryRole::Sender,
            Mode::Receive(_) => DiscoveryRole::Receiver,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DiscoveryAnnouncement {
    pub magic: [u8; 4],      // "FCAP"
    pub version: u16,        // 1
    pub role: DiscoveryRole, // 0=Sender, 1=Receiver
    pub capabilities: u32,   // Reserved for future use
    pub ip_address: [u8; 4], // IPv4 address of sender
    pub port: u16,           // TCP port (3290)
    pub timestamp: u64,      // Unix timestamp
    pub sequence: u32,       // Sequence number
    pub nonce: [u8; 32],     // Random nonce
    pub hmac: [u8; 32],      // HMAC-SHA256
}

impl DiscoveryAnnouncement {
    pub fn new(role: DiscoveryRole, ip_address: Ipv4Addr, sequence: u32) -> Self {
        let mut nonce = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        DiscoveryAnnouncement {
            magic: DISCOVERY_MAGIC,
            version: 1,
            role,
            capabilities: 0,
            ip_address: ip_address.octets(),
            port: DISCOVERY_PORT,
            timestamp,
            sequence,
            nonce,
            hmac: [0u8; 32],
        }
    }

    pub fn serialize(&self) -> [u8; ANNOUNCEMENT_SIZE] {
        let mut buf = [0u8; ANNOUNCEMENT_SIZE];
        let mut offset = 0;

        // magic (4 bytes)
        buf[offset..offset + 4].copy_from_slice(&self.magic);
        offset += 4;

        // version (2 bytes, big-endian)
        buf[offset..offset + 2].copy_from_slice(&self.version.to_be_bytes());
        offset += 2;

        // role (1 byte)
        buf[offset] = self.role as u8;
        offset += 1;

        // capabilities (4 bytes, big-endian)
        buf[offset..offset + 4].copy_from_slice(&self.capabilities.to_be_bytes());
        offset += 4;

        // ip_address (4 bytes)
        buf[offset..offset + 4].copy_from_slice(&self.ip_address);
        offset += 4;

        // port (2 bytes, big-endian)
        buf[offset..offset + 2].copy_from_slice(&self.port.to_be_bytes());
        offset += 2;

        // timestamp (8 bytes, big-endian)
        buf[offset..offset + 8].copy_from_slice(&self.timestamp.to_be_bytes());
        offset += 8;

        // sequence (4 bytes, big-endian)
        buf[offset..offset + 4].copy_from_slice(&self.sequence.to_be_bytes());
        offset += 4;

        // nonce (32 bytes)
        buf[offset..offset + 32].copy_from_slice(&self.nonce);
        offset += 32;

        // hmac (32 bytes)
        buf[offset..offset + 32].copy_from_slice(&self.hmac);

        buf
    }

    pub fn deserialize(buf: &[u8]) -> Result<Self, DiscoveryError> {
        if buf.len() < ANNOUNCEMENT_SIZE {
            return Err(DiscoveryError::HmacVerificationFailed);
        }

        let mut offset = 0;

        // magic (4 bytes)
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&buf[offset..offset + 4]);
        if magic != DISCOVERY_MAGIC {
            return Err(DiscoveryError::HmacVerificationFailed);
        }
        offset += 4;

        // version (2 bytes)
        let version = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
        offset += 2;

        // role (1 byte)
        let role = match buf[offset] {
            0 => DiscoveryRole::Sender,
            1 => DiscoveryRole::Receiver,
            _ => return Err(DiscoveryError::HmacVerificationFailed),
        };
        offset += 1;

        // capabilities (4 bytes)
        let capabilities = u32::from_be_bytes([
            buf[offset],
            buf[offset + 1],
            buf[offset + 2],
            buf[offset + 3],
        ]);
        offset += 4;

        // ip_address (4 bytes)
        let mut ip_address = [0u8; 4];
        ip_address.copy_from_slice(&buf[offset..offset + 4]);
        offset += 4;

        // port (2 bytes)
        let port = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
        offset += 2;

        // timestamp (8 bytes)
        let timestamp = u64::from_be_bytes([
            buf[offset],
            buf[offset + 1],
            buf[offset + 2],
            buf[offset + 3],
            buf[offset + 4],
            buf[offset + 5],
            buf[offset + 6],
            buf[offset + 7],
        ]);
        offset += 8;

        // sequence (4 bytes)
        let sequence = u32::from_be_bytes([
            buf[offset],
            buf[offset + 1],
            buf[offset + 2],
            buf[offset + 3],
        ]);
        offset += 4;

        // nonce (32 bytes)
        let mut nonce = [0u8; 32];
        nonce.copy_from_slice(&buf[offset..offset + 32]);
        offset += 32;

        // hmac (32 bytes)
        let mut hmac = [0u8; 32];
        hmac.copy_from_slice(&buf[offset..offset + 32]);

        Ok(DiscoveryAnnouncement {
            magic,
            version,
            role,
            capabilities,
            ip_address,
            port,
            timestamp,
            sequence,
            nonce,
            hmac,
        })
    }

    pub fn sign(&mut self, key: &[u8; 32]) {
        // Compute HMAC over all fields except the HMAC itself
        let data = self.serialize();
        let data_without_hmac = &data[..ANNOUNCEMENT_SIZE - 32];
        self.hmac = compute_hmac(key, data_without_hmac);
    }

    pub fn verify(&self, key: &[u8; 32]) -> bool {
        let data = self.serialize();
        let data_without_hmac = &data[..ANNOUNCEMENT_SIZE - 32];
        verify_hmac(key, data_without_hmac, &self.hmac)
    }

    pub fn is_timestamp_valid(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Allow timestamps within TIMESTAMP_WINDOW_SECS in either direction
        if self.timestamp > now {
            self.timestamp - now <= TIMESTAMP_WINDOW_SECS
        } else {
            now - self.timestamp <= TIMESTAMP_WINDOW_SECS
        }
    }

    pub fn get_ip_address(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.ip_address)
    }
}

/// Returns an iterator of host addresses in the subnet, excluding `local_ip`.
/// Returns None if the subnet is too large (> MAX_UNICAST_SCAN_HOSTS) or too small.
fn unicast_scan_targets(local_ip: Ipv4Addr, prefix_len: u8) -> Option<Vec<Ipv4Addr>> {
    if prefix_len > 30 || prefix_len == 0 {
        return None;
    }

    let ip_u32 = u32::from(local_ip);
    let mask = !0u32 << (32 - prefix_len);
    let network = ip_u32 & mask;
    let broadcast = network | !mask;
    let num_hosts = broadcast - network - 1;

    if num_hosts > MAX_UNICAST_SCAN_HOSTS {
        return None;
    }

    let hosts: Vec<Ipv4Addr> = ((network + 1)..broadcast)
        .filter(|&addr| addr != ip_u32)
        .map(Ipv4Addr::from)
        .collect();

    Some(hosts)
}

pub struct DiscoveryService {
    key: [u8; 32],
    role: DiscoveryRole,
    local_ip: Ipv4Addr,
    prefix_len: u8,
    cancel: Arc<AtomicBool>,
}

/// Sets the discovery cancel flag when dropped. discover_peer() holds one across its
/// awaits so that if the transfer task is aborted mid-discovery, the spawned
/// sender/receiver tasks still exit and release the discovery port.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl DiscoveryService {
    pub fn new(key: [u8; 32], mode: &Mode, local_ip: Ipv4Addr, prefix_len: u8) -> Self {
        DiscoveryService {
            key,
            role: DiscoveryRole::from(mode),
            local_ip,
            prefix_len,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Sender role: resolves with the receiver's IP once a valid announcement arrives.
    /// Receiver role: never resolves with a peer — it announces our presence and surfaces
    /// diagnostics until cancelled or dropped. The receiver's real completion signal is
    /// the sender's TCP connection (see start_shared_network_transfer): the sender stops
    /// announcing as soon as it hears us, possibly before we ever hear it, so waiting to
    /// hear the sender would deadlock on networks where UDP only works one way.
    pub async fn discover_peer<T: UI>(&self, ui: &T) -> Result<Ipv4Addr, FCError> {
        ui.output("Starting peer discovery...");

        // Stop the spawned tasks below even if this future is dropped (transfer cancelled).
        let _cancel_guard = CancelOnDrop(self.cancel.clone());

        let multicast_addr: Ipv4Addr = MULTICAST_ADDR.parse().unwrap();
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT);

        // Create multicast socket on DISCOVERY_PORT.
        // This socket receives both multicast and unicast announcements from the peer.
        // No SO_REUSEADDR: if another process holds the port we want a loud bind error,
        // not two sockets silently splitting the incoming packets.
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .map_err(|e| DiscoveryError::MulticastBindFailed(e.to_string()))?;

        socket.bind(&bind_addr.into()).map_err(|e| {
            DiscoveryError::MulticastBindFailed(format!(
                "{} (is another copy of Flying Carpet running?)",
                e
            ))
        })?;

        // Multicast setup is best-effort: on failure the unicast subnet scan still works.
        if let Err(e) = socket.join_multicast_v4(&multicast_addr, &self.local_ip) {
            println!(
                "[Discovery] Warning: could not join multicast group on {}: {}. Continuing with unicast discovery.",
                self.local_ip, e
            );
        }
        // Send multicast out the interface that owns local_ip. Without this, machines with
        // several adapters (VPNs, virtual switches) can send multicast into the wrong network.
        if let Err(e) = socket.set_multicast_if_v4(&self.local_ip) {
            println!(
                "[Discovery] Warning: could not set multicast interface to {}: {}",
                self.local_ip, e
            );
        }
        let _ = socket.set_multicast_loop_v4(false);

        socket
            .set_nonblocking(true)
            .map_err(|e| DiscoveryError::MulticastBindFailed(e.to_string()))?;

        let recv_socket = Arc::new(
            UdpSocket::from_std(socket.into())
                .map_err(|e| DiscoveryError::MulticastBindFailed(e.to_string()))?,
        );

        // Unicast socket on an ephemeral port, used only for *sending* subnet-scan
        // packets. Responses from peers arrive on the shared recv_socket (port 3290)
        // because peers reply to the discovery port, not our ephemeral source port.
        let unicast_socket = Arc::new(
            UdpSocket::bind("0.0.0.0:0")
                .await
                .map_err(|e| DiscoveryError::MulticastBindFailed(e.to_string()))?,
        );

        let multicast_dest = SocketAddrV4::new(multicast_addr, DISCOVERY_PORT);

        ui.output(&format!(
            "Searching for peer via multicast ({}) and unicast subnet scan...",
            MULTICAST_ADDR
        ));

        let (tx, mut rx) = mpsc::channel::<Ipv4Addr>(1);
        let cancel = self.cancel.clone();
        let key = self.key;
        let our_role = self.role;
        let local_ip = self.local_ip;

        // Spawn receiver task on the multicast socket.
        // Receives both multicast and direct unicast announcements on DISCOVERY_PORT.
        // There's no discovery timeout (the other device may not be started for a long
        // time), so problems are reported inline, once each, while the search continues.
        {
            let recv_socket = recv_socket.clone();
            let cancel = cancel.clone();
            let tx = tx.clone();
            let ui = ui.clone();

            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let started = std::time::Instant::now();
                let mut received_peer_packet = false;
                let mut found_sender = false;
                let mut warned_quiet = false;
                let mut warned_hmac = false;
                let mut warned_stale = false;
                let mut warned_role = false;
                loop {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }

                    match tokio::time::timeout(
                        Duration::from_millis(100),
                        recv_socket.recv_from(&mut buf),
                    )
                    .await
                    {
                        Ok(Ok((len, src_addr))) => {
                            if len < ANNOUNCEMENT_SIZE {
                                println!(
                                    "[Discovery] Ignoring {}-byte datagram from {} (too short)",
                                    len, src_addr
                                );
                                continue;
                            }

                            if let Ok(announcement) =
                                DiscoveryAnnouncement::deserialize(&buf[..len])
                            {
                                if announcement.get_ip_address() == local_ip {
                                    // our own announcement echoed back (multicast loopback)
                                    continue;
                                }
                                received_peer_packet = true;
                                if !announcement.verify(&key) {
                                    if !warned_hmac {
                                        warned_hmac = true;
                                        ui.output("Received an announcement that failed authentication. If it came from the other device, check that the password matches on both. Still searching...");
                                    }
                                    println!(
                                        "[Discovery] Announcement from {} failed HMAC check (different password?)",
                                        src_addr
                                    );
                                    continue;
                                }
                                if !announcement.is_timestamp_valid() {
                                    if !warned_stale {
                                        warned_stale = true;
                                        ui.output("Received an announcement with an out-of-date timestamp. Check that both devices' clocks are set correctly. Still searching...");
                                    }
                                    println!(
                                        "[Discovery] Announcement from {} has a stale timestamp (clocks out of sync?)",
                                        src_addr
                                    );
                                    continue;
                                }
                                if announcement.role == our_role {
                                    if !warned_role {
                                        warned_role = true;
                                        ui.output("Received an announcement from a device in the same mode as this one. If it's the other device of this transfer, one side must select Send and the other Receive. Still searching...");
                                    }
                                    println!(
                                        "[Discovery] Announcement from {} has our own role, ignoring",
                                        src_addr
                                    );
                                    continue;
                                }

                                println!(
                                    "[Discovery] Valid peer announcement from {} (source {})",
                                    announcement.get_ip_address(),
                                    src_addr
                                );
                                if our_role == DiscoveryRole::Receiver {
                                    // The receiver's completion signal is the sender's TCP
                                    // connection, not discovery. Keep announcing so the
                                    // sender can find us; this is informational only.
                                    if !found_sender {
                                        found_sender = true;
                                        ui.output(&format!(
                                            "Found the sender at {}. Waiting for it to connect...",
                                            announcement.get_ip_address()
                                        ));
                                    }
                                    continue;
                                }
                                let _ = tx.send(announcement.get_ip_address()).await;
                                break;
                            } else {
                                println!(
                                    "[Discovery] Unparseable {}-byte datagram from {}",
                                    len, src_addr
                                );
                            }
                        }
                        Ok(Err(e)) => {
                            // e.g. WSAECONNRESET on Windows after an ICMP unreachable; not fatal
                            println!("[Discovery] recv error: {}", e);
                        }
                        Err(_) => {
                            // 100ms poll timeout: check cancellation and continue. If nothing
                            // from the peer has arrived after a while, hint at likely causes
                            // (the other device may also simply not be started yet).
                            if !warned_quiet
                                && !received_peer_packet
                                && started.elapsed().as_secs() >= 30
                            {
                                warned_quiet = true;
                                ui.output("Still searching. If the other device has already started the transfer, check that both devices are on the same network and that no firewall is blocking UDP port 3290.");
                            }
                        }
                    }
                }
            });
        }

        // Spawn multicast sender task
        {
            let socket = recv_socket.clone();
            let cancel = cancel.clone();

            tokio::spawn(async move {
                let mut sequence = 0u32;
                let mut send_interval = interval(Duration::from_millis(DISCOVERY_INTERVAL_MS));
                loop {
                    send_interval.tick().await;
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }

                    let mut announcement = DiscoveryAnnouncement::new(our_role, local_ip, sequence);
                    announcement.sign(&key);
                    let data = announcement.serialize();

                    let _ = socket.send_to(&data, SocketAddr::V4(multicast_dest)).await;
                    sequence = sequence.wrapping_add(1);
                }
            });
        }

        // Spawn unicast sender task (subnet scan) if subnet is small enough
        if let Some(targets) = unicast_scan_targets(self.local_ip, self.prefix_len) {
            let cancel = cancel.clone();
            ui.output(&format!(
                "Scanning {} addresses on the local /{} subnet...",
                targets.len(),
                self.prefix_len
            ));

            tokio::spawn(async move {
                let mut sequence = 0u32;
                loop {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }

                    for &target_ip in &targets {
                        if cancel.load(Ordering::SeqCst) {
                            return;
                        }

                        let mut announcement =
                            DiscoveryAnnouncement::new(our_role, local_ip, sequence);
                        announcement.sign(&key);
                        let data = announcement.serialize();

                        let dest = SocketAddrV4::new(target_ip, DISCOVERY_PORT);
                        let _ = unicast_socket.send_to(&data, SocketAddr::V4(dest)).await;
                    }

                    sequence = sequence.wrapping_add(1);
                    tokio::time::sleep(Duration::from_millis(DISCOVERY_INTERVAL_MS)).await;
                }
            });
        } else {
            ui.output("Subnet too large for unicast scan, relying on multicast only.");
        }

        // Wait for peer discovery. There's deliberately no timeout: the user may start
        // this side long before the other, so keep searching until the peer appears or
        // the transfer is cancelled. In the receiver role nothing is ever sent on the
        // channel, so this waits (while the tasks above keep announcing) until the
        // future is cancelled or dropped when the sender's TCP connection arrives.
        let result: Result<Ipv4Addr, FCError> = async {
            loop {
                tokio::select! {
                    Some(peer_ip) = rx.recv() => {
                        return Ok(peer_ip);
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        if cancel.load(Ordering::SeqCst) {
                            return Err(FCError {
                                message: "Discovery cancelled".to_string(),
                            });
                        }
                    }
                }
            }
        }
        .await;

        // Stop all background tasks
        self.cancel.store(true, Ordering::SeqCst);

        match result {
            Ok(peer_ip) => {
                ui.output(&format!("Discovered peer at {}", peer_ip));
                Ok(peer_ip)
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_announcement_serialize_deserialize() {
        let ip = Ipv4Addr::new(192, 168, 1, 100);
        let mut announcement = DiscoveryAnnouncement::new(DiscoveryRole::Sender, ip, 42);

        let key = [0xABu8; 32];
        announcement.sign(&key);

        let serialized = announcement.serialize();
        assert_eq!(serialized.len(), ANNOUNCEMENT_SIZE);

        let deserialized = DiscoveryAnnouncement::deserialize(&serialized).unwrap();
        assert_eq!(deserialized.magic, DISCOVERY_MAGIC);
        assert_eq!(deserialized.version, 1);
        assert_eq!(deserialized.role, DiscoveryRole::Sender);
        assert_eq!(deserialized.ip_address, ip.octets());
        assert_eq!(deserialized.port, DISCOVERY_PORT);
        assert_eq!(deserialized.sequence, 42);
        assert!(deserialized.verify(&key));
    }

    // Known-answer test shared with the Android implementation
    // (DiscoveryUnitTest.kt) to guarantee the wire formats match.
    #[test]
    fn test_cross_platform_vector() {
        let mut announcement = DiscoveryAnnouncement {
            magic: DISCOVERY_MAGIC,
            version: 1,
            role: DiscoveryRole::Receiver,
            capabilities: 0,
            ip_address: [192, 168, 1, 42],
            port: DISCOVERY_PORT,
            timestamp: 1750000000,
            sequence: 7,
            nonce: [0x11; 32],
            hmac: [0; 32],
        };
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        announcement.sign(&key);
        let serialized = announcement.serialize();
        let hex: String = serialized.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "4643415000010100000000c0a8012a0cda00000000684ee180000000071111111111111111111111111111111111111111111111111111111111111111adc75d44854c84be1627ef8933f16d0fcb26807fccb7562ea3609f13982f7a9a"
        );
        assert!(announcement.verify(&key));
    }

    #[test]
    fn test_hmac_verification() {
        let ip = Ipv4Addr::new(10, 0, 0, 5);
        let mut announcement = DiscoveryAnnouncement::new(DiscoveryRole::Receiver, ip, 1);

        let key = [0xCDu8; 32];
        announcement.sign(&key);

        assert!(announcement.verify(&key));

        // Wrong key should fail
        let wrong_key = [0xEFu8; 32];
        assert!(!announcement.verify(&wrong_key));
    }

    #[test]
    fn test_timestamp_validation() {
        let ip = Ipv4Addr::new(172, 16, 0, 1);
        let announcement = DiscoveryAnnouncement::new(DiscoveryRole::Sender, ip, 0);

        assert!(announcement.is_timestamp_valid());
    }

    #[test]
    fn test_unicast_scan_targets_24() {
        let ip = Ipv4Addr::new(192, 168, 1, 100);
        let targets = unicast_scan_targets(ip, 24).unwrap();
        assert_eq!(targets.len(), 253); // 254 hosts minus our own
        assert!(!targets.contains(&ip));
        assert!(targets.contains(&Ipv4Addr::new(192, 168, 1, 1)));
        assert!(targets.contains(&Ipv4Addr::new(192, 168, 1, 254)));
        assert!(!targets.contains(&Ipv4Addr::new(192, 168, 1, 0))); // network
        assert!(!targets.contains(&Ipv4Addr::new(192, 168, 1, 255))); // broadcast
    }

    #[test]
    fn test_unicast_scan_targets_too_large() {
        let ip = Ipv4Addr::new(10, 0, 0, 1);
        // /16 = 65534 hosts, way over MAX_UNICAST_SCAN_HOSTS
        assert!(unicast_scan_targets(ip, 16).is_none());
    }

    #[test]
    fn test_unicast_scan_targets_small() {
        let ip = Ipv4Addr::new(10, 0, 0, 1);
        let targets = unicast_scan_targets(ip, 30).unwrap();
        assert_eq!(targets.len(), 1); // /30 = 2 hosts, minus ours = 1
    }
}
