use crate::beacon::{decode_beacon, encode_beacon, BeaconMessage};
use crate::error::{DiscoveryError, Result};
use crate::peer::{DiscoveredPeer, PeerDirectory};
use crate::service::DiscoveryEvent;
use bridge_core::{Capabilities, DeviceType, NodeId, ProtocolVersion};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, watch, RwLock};
use tracing::{debug, info, warn};

pub const DEFAULT_UDP_BROADCAST_PORT: u16 = 42424;
pub const DEFAULT_BEACON_INTERVAL: Duration = Duration::from_secs(2);
pub const DEFAULT_BEACON_TTL: Duration = Duration::from_secs(6);
pub const DEFAULT_BEACON_PRUNE_INTERVAL: Duration = Duration::from_secs(1);

/// Configuration options for UDP broadcast beacon discovery.
#[derive(Debug, Clone)]
pub struct UdpConfig {
    pub broadcast_port: u16,
    pub bind_addr: SocketAddr,
    pub broadcast_targets: Vec<SocketAddr>,
    pub beacon_interval: Duration,
    pub ttl: Duration,
    pub prune_interval: Duration,
    pub query_on_start: bool,
    pub goodbye_on_shutdown: bool,
}

impl Default for UdpConfig {
    fn default() -> Self {
        let broadcast_port = DEFAULT_UDP_BROADCAST_PORT;
        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), broadcast_port);
        let broadcast_targets = vec![SocketAddr::new(
            IpAddr::V4(Ipv4Addr::BROADCAST),
            broadcast_port,
        )];

        Self {
            broadcast_port,
            bind_addr,
            broadcast_targets,
            beacon_interval: DEFAULT_BEACON_INTERVAL,
            ttl: DEFAULT_BEACON_TTL,
            prune_interval: DEFAULT_BEACON_PRUNE_INTERVAL,
            query_on_start: true,
            goodbye_on_shutdown: true,
        }
    }
}

#[derive(Debug, Clone)]
struct LocalAdvertisement {
    node_id: NodeId,
    device_name: String,
    device_type: DeviceType,
    port: u16,
    capabilities: Capabilities,
    seq: u64,
}

/// Daemon managing UDP broadcast beacon publication, probe responses, and peer directory freshness.
pub struct UdpDiscovery {
    config: UdpConfig,
    directory: PeerDirectory,
    local_node_id: Arc<RwLock<Option<NodeId>>>,
    advertising_info: Arc<RwLock<Option<LocalAdvertisement>>>,
    event_tx: broadcast::Sender<DiscoveryEvent>,
    shutdown_tx: Option<watch::Sender<bool>>,
    task_handles: Vec<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for UdpDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdpDiscovery")
            .field("config", &self.config)
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

impl UdpDiscovery {
    /// Creates a new `UdpDiscovery` instance with its own peer directory and event bus.
    pub fn new(config: UdpConfig) -> Result<Self> {
        let (event_tx, _) = broadcast::channel(128);
        Ok(Self {
            config,
            directory: PeerDirectory::new(),
            local_node_id: Arc::new(RwLock::new(None)),
            advertising_info: Arc::new(RwLock::new(None)),
            event_tx,
            shutdown_tx: None,
            task_handles: Vec::new(),
        })
    }

    /// Creates a new `UdpDiscovery` instance sharing a peer directory and event bus.
    pub fn with_directory_and_events(
        config: UdpConfig,
        directory: PeerDirectory,
        event_tx: broadcast::Sender<DiscoveryEvent>,
    ) -> Self {
        Self {
            config,
            directory,
            local_node_id: Arc::new(RwLock::new(None)),
            advertising_info: Arc::new(RwLock::new(None)),
            event_tx,
            shutdown_tx: None,
            task_handles: Vec::new(),
        }
    }

    /// Accesses the underlying peer directory.
    pub fn directory(&self) -> &PeerDirectory {
        &self.directory
    }

    /// Subscribes to peer discovery and presence lifecycle events.
    pub fn subscribe(&self) -> broadcast::Receiver<DiscoveryEvent> {
        self.event_tx.subscribe()
    }

    /// Advertises this local node on the network via UDP broadcast beacons.
    pub async fn advertise(
        &self,
        node_id: NodeId,
        device_name: &str,
        device_type: DeviceType,
        port: u16,
        capabilities: Capabilities,
    ) -> Result<()> {
        let mut local_id = self.local_node_id.write().await;
        *local_id = Some(node_id);

        let mut adv = self.advertising_info.write().await;
        *adv = Some(LocalAdvertisement {
            node_id,
            device_name: device_name.to_string(),
            device_type,
            port,
            capabilities,
            seq: 1,
        });

        info!(node_id = %node_id, port, "Configured BridgeOS UDP broadcast beacon advertisement");
        Ok(())
    }

    /// Stops advertising this local node via UDP broadcast.
    pub async fn stop_advertising(&self) -> Result<()> {
        let mut adv = self.advertising_info.write().await;
        if adv.take().is_some() {
            info!("Stopped BridgeOS UDP broadcast beacon advertisement");
        }
        Ok(())
    }

    /// Creates and configures the UDP broadcast socket with SO_REUSEADDR.
    fn create_udp_socket(bind_addr: SocketAddr) -> Result<UdpSocket> {
        let domain = if bind_addr.is_ipv4() {
            socket2::Domain::IPV4
        } else {
            socket2::Domain::IPV6
        };

        let socket =
            socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
                .map_err(|e| {
                    DiscoveryError::Internal(format!("failed to create UDP socket: {e}"))
                })?;

        socket
            .set_reuse_address(true)
            .map_err(|e| DiscoveryError::Internal(format!("failed to set SO_REUSEADDR: {e}")))?;

        #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
        let _ = socket.set_reuse_port(true);

        socket
            .set_broadcast(true)
            .map_err(|e| DiscoveryError::Internal(format!("failed to enable broadcast: {e}")))?;

        socket
            .set_nonblocking(true)
            .map_err(|e| DiscoveryError::Internal(format!("failed to set nonblocking: {e}")))?;

        socket.bind(&bind_addr.into()).map_err(|e| {
            DiscoveryError::Internal(format!("failed to bind UDP socket to {bind_addr}: {e}"))
        })?;

        let std_socket: std::net::UdpSocket = socket.into();
        UdpSocket::from_std(std_socket).map_err(|e| {
            DiscoveryError::Internal(format!("failed to convert into Tokio UdpSocket: {e}"))
        })
    }

    /// Starts the background UDP broadcast listener and beacon emitter.
    pub fn start_discovery(&mut self) -> Result<()> {
        if self.shutdown_tx.is_some() {
            return Err(DiscoveryError::AlreadyRunning);
        }

        let socket = Arc::new(Self::create_udp_socket(self.config.bind_addr)?);
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let directory = self.directory.clone();
        let local_node_id = self.local_node_id.clone();
        let advertising_info = self.advertising_info.clone();
        let event_tx = self.event_tx.clone();
        let config = self.config.clone();
        let socket_recv = socket.clone();
        let socket_send = socket.clone();

        // 1. If configured and local identity is known, broadcast a Query probe immediately
        if config.query_on_start {
            let socket_query = socket.clone();
            let local_id_clone = local_node_id.clone();
            let targets = config.broadcast_targets.clone();
            tokio::spawn(async move {
                let sender_id = {
                    let guard = local_id_clone.read().await;
                    guard.unwrap_or_else(|| NodeId::from_bytes([0u8; 32]))
                };
                let query = BeaconMessage::Query { sender_id, seq: 1 };
                if let Ok(bytes) = encode_beacon(&query) {
                    for target in targets {
                        let _ = socket_query.send_to(&bytes, target).await;
                    }
                    debug!(sender_id = %sender_id, "Sent initial UDP discovery Query probe");
                }
            });
        }

        // 2. Spawn main discovery worker loop
        let handle = tokio::spawn(async move {
            let mut prune_ticker = tokio::time::interval(config.prune_interval);
            prune_ticker.tick().await; // Skip initial tick

            let mut beacon_ticker = tokio::time::interval(config.beacon_interval);
            beacon_ticker.tick().await; // Skip initial tick

            let mut buf = [0u8; 2048];

            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            debug!("UDP discovery worker received shutdown signal");
                            break;
                        }
                    }

                    _ = prune_ticker.tick() => {
                        let expired = directory.prune_expired(config.ttl);
                        for peer in expired {
                            info!(node_id = %peer.node_id, "Peer expired after UDP beacon TTL timeout");
                            let _ = event_tx.send(DiscoveryEvent::PeerExpired(peer.node_id));
                        }
                    }

                    _ = beacon_ticker.tick() => {
                        let adv_opt = {
                            let mut adv_guard = advertising_info.write().await;
                            if let Some(ref mut adv) = *adv_guard {
                                adv.seq = adv.seq.wrapping_add(1);
                                Some(adv.clone())
                            } else {
                                None
                            }
                        };

                        if let Some(adv) = adv_opt {
                            let msg = BeaconMessage::Announcement {
                                version: ProtocolVersion::CURRENT,
                                node_id: adv.node_id,
                                device_name: adv.device_name,
                                device_type: adv.device_type,
                                port: adv.port,
                                capabilities: adv.capabilities,
                                seq: adv.seq,
                            };

                            if let Ok(encoded) = encode_beacon(&msg) {
                                for target in &config.broadcast_targets {
                                    if let Err(e) = socket_send.send_to(&encoded, target).await {
                                        debug!(target = %target, error = %e, "Failed to send UDP broadcast beacon");
                                    }
                                }
                            }
                        }
                    }

                    recv_res = socket_recv.recv_from(&mut buf) => {
                        match recv_res {
                            Ok((len, src_addr)) => {
                                match decode_beacon(&buf[..len]) {
                                    Ok(beacon) => {
                                        let local_id = {
                                            let guard = local_node_id.read().await;
                                            *guard
                                        };

                                        // Ignore packets from ourselves
                                        if let Some(self_id) = local_id {
                                            if beacon.node_id() == self_id {
                                                continue;
                                            }
                                        }

                                        match beacon {
                                            BeaconMessage::Announcement {
                                                node_id,
                                                device_name,
                                                device_type,
                                                port,
                                                capabilities,
                                                ..
                                            } => {
                                                // Resolve actual routable address: sender IP + advertised service port
                                                let peer_addr = SocketAddr::new(src_addr.ip(), port);
                                                let discovered = DiscoveredPeer::new(
                                                    node_id,
                                                    device_name.clone(),
                                                    device_type,
                                                    vec![peer_addr],
                                                    capabilities,
                                                );

                                                let (peer, is_new) = directory.insert_or_update(discovered);
                                                if is_new {
                                                    info!(
                                                        node_id = %peer.node_id,
                                                        name = %peer.device_name,
                                                        addr = %peer_addr,
                                                        "Discovered new BridgeOS peer via UDP broadcast"
                                                    );
                                                    let _ = event_tx.send(DiscoveryEvent::PeerDiscovered(peer));
                                                } else {
                                                    debug!(node_id = %peer.node_id, "Refreshed BridgeOS peer via UDP beacon");
                                                    let _ = event_tx.send(DiscoveryEvent::PeerUpdated(peer));
                                                }
                                            }

                                            BeaconMessage::Goodbye { node_id, .. } => {
                                                if directory.remove(&node_id).is_some() {
                                                    info!(node_id = %node_id, "Peer explicitly departed via UDP goodbye");
                                                    let _ = event_tx.send(DiscoveryEvent::PeerLost(node_id));
                                                }
                                            }

                                            BeaconMessage::Query { sender_id, .. } => {
                                                debug!(sender_id = %sender_id, "Received UDP discovery Query probe");
                                                // If we are advertising, respond immediately with an Announcement
                                                let adv_opt = {
                                                    let guard = advertising_info.read().await;
                                                    guard.clone()
                                                };

                                                if let Some(adv) = adv_opt {
                                                    let reply = BeaconMessage::Announcement {
                                                        version: ProtocolVersion::CURRENT,
                                                        node_id: adv.node_id,
                                                        device_name: adv.device_name,
                                                        device_type: adv.device_type,
                                                        port: adv.port,
                                                        capabilities: adv.capabilities,
                                                        seq: adv.seq,
                                                    };
                                                    if let Ok(encoded) = encode_beacon(&reply) {
                                                        // Direct reply to the sender
                                                        let _ = socket_send.send_to(&encoded, src_addr).await;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        debug!(src = %src_addr, error = %err, "Discarded invalid or unrecognized UDP packet");
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "UDP socket recv_from error");
                                break;
                            }
                        }
                    }
                }
            }
        });

        self.task_handles.push(handle);
        Ok(())
    }

    /// Broadcasts an explicit Goodbye beacon to peers and shuts down background tasks.
    pub async fn shutdown(&mut self) -> Result<()> {
        if self.config.goodbye_on_shutdown {
            let adv_opt = {
                let guard = self.advertising_info.read().await;
                guard.clone()
            };
            if let Some(adv) = adv_opt {
                let goodbye = BeaconMessage::Goodbye {
                    node_id: adv.node_id,
                    seq: adv.seq.wrapping_add(1),
                };
                if let Ok(encoded) = encode_beacon(&goodbye) {
                    // Try to send via an ephemeral broadcast socket
                    if let Ok(send_sock) = UdpSocket::bind("0.0.0.0:0").await {
                        let _ = send_sock.set_broadcast(true);
                        for target in &self.config.broadcast_targets {
                            let _ = send_sock.send_to(&encoded, target).await;
                        }
                    }
                }
            }
        }

        let _ = self.stop_advertising().await;

        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(true);
        }

        for handle in self.task_handles.drain(..) {
            let _ = handle.await;
        }

        info!("UdpDiscovery shutdown complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_udp_discovery_config_defaults() {
        let config = UdpConfig::default();
        assert_eq!(config.broadcast_port, DEFAULT_UDP_BROADCAST_PORT);
        assert_eq!(config.broadcast_targets.len(), 1);
        assert_eq!(
            config.broadcast_targets[0].port(),
            DEFAULT_UDP_BROADCAST_PORT
        );
        assert!(config.query_on_start);
        assert!(config.goodbye_on_shutdown);
    }

    #[tokio::test]
    async fn test_udp_discovery_advertise_and_stop() {
        let config = UdpConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            broadcast_targets: vec!["127.0.0.1:0".parse().unwrap()],
            ..UdpConfig::default()
        };

        let discovery = UdpDiscovery::new(config).expect("create UdpDiscovery");
        let node_id = NodeId::from_bytes([0x77; 32]);
        let mut caps = Capabilities::default();
        caps.insert(Capabilities::FILE_TRANSFER);

        discovery
            .advertise(node_id, "TestPC", DeviceType::Windows, 9000, caps)
            .await
            .expect("advertise");

        {
            let guard = discovery.advertising_info.read().await;
            let adv = guard.as_ref().expect("advertisement should be present");
            assert_eq!(adv.node_id, node_id);
            assert_eq!(adv.device_name, "TestPC");
            assert_eq!(adv.port, 9000);
            assert_eq!(adv.capabilities, caps);
        }

        discovery
            .stop_advertising()
            .await
            .expect("stop advertising");

        {
            let guard = discovery.advertising_info.read().await;
            assert!(guard.is_none());
        }
    }

    #[tokio::test]
    async fn test_udp_discovery_create_and_shutdown() {
        let config = UdpConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            broadcast_targets: vec!["127.0.0.1:0".parse().unwrap()],
            beacon_interval: Duration::from_millis(50),
            ttl: Duration::from_millis(200),
            prune_interval: Duration::from_millis(50),
            query_on_start: false,
            goodbye_on_shutdown: false,
            ..UdpConfig::default()
        };

        let mut discovery = UdpDiscovery::new(config).expect("create discovery");
        discovery.start_discovery().expect("start discovery");
        assert!(discovery.directory().is_empty());
        discovery.shutdown().await.expect("shutdown discovery");
    }
}
