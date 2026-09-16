use crate::error::{DiscoveryError, Result};
use crate::peer::{DiscoveredPeer, PeerDirectory};
use bridge_core::{Capabilities, DeviceType, NodeId, ProtocolVersion};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, watch, Mutex, RwLock};
use tracing::{debug, info, warn};

pub const BRIDGEOS_SERVICE_TYPE: &str = "_bridgeos._tcp.local.";
pub const DEFAULT_TTL: Duration = Duration::from_secs(5);
pub const DEFAULT_PRUNE_INTERVAL: Duration = Duration::from_secs(1);

/// Configuration options for mDNS peer discovery and publication.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    pub service_type: String,
    pub ttl: Duration,
    pub prune_interval: Duration,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            service_type: BRIDGEOS_SERVICE_TYPE.to_string(),
            ttl: DEFAULT_TTL,
            prune_interval: DEFAULT_PRUNE_INTERVAL,
        }
    }
}

/// Events emitted when peer discovery status changes on the local network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A new peer was discovered.
    PeerDiscovered(DiscoveredPeer),
    /// An existing peer refreshed its metadata or addresses.
    PeerUpdated(DiscoveredPeer),
    /// A peer explicitly announced service departure via mDNS goodbye.
    PeerLost(NodeId),
    /// A peer stopped heartbeating and expired after TTL.
    PeerExpired(NodeId),
}

/// Constructs TXT record properties for advertising a BridgeOS node.
pub fn peer_to_txt_properties(
    node_id: &NodeId,
    device_name: &str,
    device_type: DeviceType,
    capabilities: Capabilities,
) -> HashMap<String, String> {
    let mut props = HashMap::new();
    props.insert("node_id".to_string(), node_id.to_hex());
    props.insert("name".to_string(), device_name.to_string());
    props.insert("type".to_string(), device_type.to_string());
    props.insert("caps".to_string(), format!("{:x}", capabilities.flags));
    props.insert("ver".to_string(), ProtocolVersion::CURRENT.to_string());
    props
}

/// Internal helper to parse peer fields extracted from TXT records and network metadata.
pub fn parse_discovered_peer(
    node_id_hex: Option<&str>,
    device_name: Option<&str>,
    device_type_str: Option<&str>,
    capabilities_str: Option<&str>,
    addresses: Vec<SocketAddr>,
    fallback_name: &str,
) -> Result<DiscoveredPeer> {
    let node_id_hex = node_id_hex
        .ok_or_else(|| DiscoveryError::InvalidTxtRecord("missing 'node_id' in TXT".into()))?;

    let node_id = NodeId::from_hex(node_id_hex)
        .map_err(|e| DiscoveryError::InvalidNodeId(format!("invalid hex in 'node_id': {e}")))?;

    let device_name = device_name.unwrap_or(fallback_name).to_string();

    let device_type = device_type_str.map_or(DeviceType::Unknown, |s| {
        DeviceType::from_str(s).unwrap_or(DeviceType::Unknown)
    });

    let capabilities = if let Some(caps_str) = capabilities_str {
        u32::from_str_radix(caps_str, 16)
            .map(Capabilities::from_bits)
            .unwrap_or_default()
    } else {
        Capabilities::default()
    };

    Ok(DiscoveredPeer::new(
        node_id,
        device_name,
        device_type,
        addresses,
        capabilities,
    ))
}

/// Parses an mDNS `ServiceInfo` into a `DiscoveredPeer`.
pub fn parse_service_info(info: &ServiceInfo) -> Result<DiscoveredPeer> {
    let port = info.get_port();
    let addresses: Vec<SocketAddr> = info
        .get_addresses()
        .iter()
        .map(|ip| SocketAddr::new(*ip, port))
        .collect();

    parse_discovered_peer(
        info.get_property_val_str("node_id"),
        info.get_property_val_str("name"),
        info.get_property_val_str("type"),
        info.get_property_val_str("caps"),
        addresses,
        info.get_fullname(),
    )
}

/// Parses an mDNS `ResolvedService` into a `DiscoveredPeer`.
pub fn parse_resolved_service(info: &mdns_sd::ResolvedService) -> Result<DiscoveredPeer> {
    let port = info.get_port();
    let addresses: Vec<SocketAddr> = info
        .get_addresses()
        .iter()
        .map(|scoped_ip| SocketAddr::new(scoped_ip.to_ip_addr(), port))
        .collect();

    parse_discovered_peer(
        info.get_property_val_str("node_id"),
        info.get_property_val_str("name"),
        info.get_property_val_str("type"),
        info.get_property_val_str("caps"),
        addresses,
        info.get_fullname(),
    )
}

/// Daemon managing mDNS registration, service browsing, and peer directory freshness.
pub struct MdnsDiscovery {
    daemon: ServiceDaemon,
    config: DiscoveryConfig,
    directory: PeerDirectory,
    fullname_to_node: Arc<RwLock<HashMap<String, NodeId>>>,
    local_node_id: Arc<RwLock<Option<NodeId>>>,
    registered_fullname: Arc<Mutex<Option<String>>>,
    event_tx: broadcast::Sender<DiscoveryEvent>,
    shutdown_tx: Option<watch::Sender<bool>>,
    task_handles: Vec<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for MdnsDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdnsDiscovery")
            .field("config", &self.config)
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

impl MdnsDiscovery {
    /// Creates a new `MdnsDiscovery` instance.
    pub fn new(config: DiscoveryConfig) -> Result<Self> {
        let daemon = ServiceDaemon::new()
            .map_err(|e| DiscoveryError::Mdns(format!("failed to create ServiceDaemon: {e}")))?;
        let (event_tx, _) = broadcast::channel(128);

        Ok(Self {
            daemon,
            config,
            directory: PeerDirectory::new(),
            fullname_to_node: Arc::new(RwLock::new(HashMap::new())),
            local_node_id: Arc::new(RwLock::new(None)),
            registered_fullname: Arc::new(Mutex::new(None)),
            event_tx,
            shutdown_tx: None,
            task_handles: Vec::new(),
        })
    }

    /// Accesses the underlying peer directory.
    pub fn directory(&self) -> &PeerDirectory {
        &self.directory
    }

    /// Subscribes to peer discovery and lifecycle events.
    pub fn subscribe(&self) -> broadcast::Receiver<DiscoveryEvent> {
        self.event_tx.subscribe()
    }

    /// Advertises this local node on the network via mDNS.
    pub async fn advertise(
        &self,
        node_id: NodeId,
        device_name: &str,
        device_type: DeviceType,
        port: u16,
        capabilities: Capabilities,
        ips: Option<Vec<IpAddr>>,
    ) -> Result<()> {
        let mut reg_lock = self.registered_fullname.lock().await;
        if reg_lock.is_some() {
            return Err(DiscoveryError::AlreadyRunning);
        }

        // Set local node id so we ignore our own announcements in browsing
        {
            let mut local_id = self.local_node_id.write().await;
            *local_id = Some(node_id);
        }

        let properties = peer_to_txt_properties(&node_id, device_name, device_type, capabilities);
        let instance_name = format!(
            "{}-{}",
            device_name.replace(['.', ' '], "-"),
            &node_id.to_hex()[..8]
        );
        let host_name = format!("{}.local.", &node_id.to_hex()[..12]);

        let service_info = if let Some(custom_ips) = ips {
            let ip_strs: Vec<String> = custom_ips.iter().map(ToString::to_string).collect();
            let ip_slices: Vec<&str> = ip_strs.iter().map(String::as_str).collect();
            ServiceInfo::new(
                &self.config.service_type,
                &instance_name,
                &host_name,
                ip_slices.as_slice(),
                port,
                properties,
            )
        } else {
            ServiceInfo::new(
                &self.config.service_type,
                &instance_name,
                &host_name,
                "",
                port,
                properties,
            )
        }
        .map_err(|e| DiscoveryError::Mdns(format!("failed to construct ServiceInfo: {e}")))?;

        let fullname = service_info.get_fullname().to_string();
        self.daemon
            .register(service_info)
            .map_err(|e| DiscoveryError::Mdns(format!("failed to register mDNS service: {e}")))?;

        info!(node_id = %node_id, fullname = %fullname, "Advertised BridgeOS mDNS service");
        *reg_lock = Some(fullname);
        Ok(())
    }

    /// Unregisters the advertised service.
    pub async fn stop_advertising(&self) -> Result<()> {
        let mut reg_lock = self.registered_fullname.lock().await;
        if let Some(fullname) = reg_lock.take() {
            self.daemon.unregister(&fullname).map_err(|e| {
                DiscoveryError::Mdns(format!("failed to unregister {fullname}: {e}"))
            })?;
            info!(fullname = %fullname, "Unregistered BridgeOS mDNS service");
        }
        Ok(())
    }

    /// Starts active discovery of peers on the local LAN.
    pub fn start_discovery(&mut self) -> Result<()> {
        if self.shutdown_tx.is_some() {
            return Err(DiscoveryError::AlreadyRunning);
        }

        let receiver = self
            .daemon
            .browse(&self.config.service_type)
            .map_err(|e| DiscoveryError::Mdns(format!("failed to start browsing: {e}")))?;

        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let directory = self.directory.clone();
        let fullname_to_node = self.fullname_to_node.clone();
        let local_node_id = self.local_node_id.clone();
        let event_tx = self.event_tx.clone();
        let prune_interval = self.config.prune_interval;
        let ttl = self.config.ttl;

        // Background worker loop
        let handle = tokio::spawn(async move {
            let mut prune_ticker = tokio::time::interval(prune_interval);
            // Skip first instant tick
            prune_ticker.tick().await;

            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            debug!("mDNS discovery background worker received shutdown signal");
                            break;
                        }
                    }

                    _ = prune_ticker.tick() => {
                        let expired = directory.prune_expired(ttl);
                        for peer in expired {
                            info!(node_id = %peer.node_id, "Peer expired after heartbeat TTL timeout");
                            let _ = event_tx.send(DiscoveryEvent::PeerExpired(peer.node_id));
                        }
                    }

                    event_result = receiver.recv_async() => {
                        match event_result {
                            Ok(event) => {
                                match event {
                                    ServiceEvent::ServiceResolved(info) => {
                                        match parse_resolved_service(&info) {
                                            Ok(peer) => {
                                                // Ignore self announcements
                                                let is_self = {
                                                    let guard = local_node_id.read().await;
                                                    *guard == Some(peer.node_id)
                                                };

                                                if is_self {
                                                    continue;
                                                }

                                                let fullname = info.get_fullname().to_string();
                                                {
                                                    let mut names = fullname_to_node.write().await;
                                                    names.insert(fullname, peer.node_id);
                                                }

                                                let (discovered_peer, is_new) = directory.insert_or_update(peer);
                                                if is_new {
                                                    info!(node_id = %discovered_peer.node_id, name = %discovered_peer.device_name, "Discovered new BridgeOS peer");
                                                    let _ = event_tx.send(DiscoveryEvent::PeerDiscovered(discovered_peer));
                                                } else {
                                                    debug!(node_id = %discovered_peer.node_id, "Updated discovered BridgeOS peer");
                                                    let _ = event_tx.send(DiscoveryEvent::PeerUpdated(discovered_peer));
                                                }
                                            }
                                            Err(err) => {
                                                warn!(error = %err, "Failed to parse BridgeOS service TXT record");
                                            }
                                        }
                                    }
                                    ServiceEvent::ServiceRemoved(_service_type, fullname) => {
                                        let node_id_opt = {
                                            let mut names = fullname_to_node.write().await;
                                            names.remove(&fullname)
                                        };

                                        if let Some(node_id) = node_id_opt {
                                            if directory.remove(&node_id).is_some() {
                                                info!(node_id = %node_id, fullname = %fullname, "Peer explicitly departed (ServiceRemoved)");
                                                let _ = event_tx.send(DiscoveryEvent::PeerLost(node_id));
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "mDNS event receiver channel closed or error");
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

    /// Shuts down all advertising, browsing, and background tasks.
    pub async fn shutdown(&mut self) -> Result<()> {
        let _ = self.stop_advertising().await;

        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(true);
        }

        for handle in self.task_handles.drain(..) {
            let _ = handle.await;
        }

        let _ = self.daemon.stop_browse(&self.config.service_type);
        let _ = self.daemon.shutdown();
        info!("MdnsDiscovery shutdown complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_txt_record_serialization_and_parsing() {
        let node_id = NodeId::from_bytes([7u8; 32]);
        let mut caps = Capabilities::default();
        caps.insert(Capabilities::FILE_TRANSFER);
        caps.insert(Capabilities::CLIPBOARD_TEXT);

        let props = peer_to_txt_properties(&node_id, "DevPC", DeviceType::Windows, caps);
        assert_eq!(props.get("node_id").unwrap(), &node_id.to_hex());
        assert_eq!(props.get("name").unwrap(), "DevPC");
        assert_eq!(props.get("type").unwrap(), "Windows");
        assert_eq!(props.get("caps").unwrap(), &format!("{:x}", caps.flags));

        let service_info = ServiceInfo::new(
            BRIDGEOS_SERVICE_TYPE,
            "DevPC-Node",
            "devpc.local.",
            "127.0.0.1",
            9000,
            props,
        )
        .expect("should create ServiceInfo");

        let peer = parse_service_info(&service_info).expect("should parse service info");
        assert_eq!(peer.node_id, node_id);
        assert_eq!(peer.device_name, "DevPC");
        assert_eq!(peer.device_type, DeviceType::Windows);
        assert_eq!(peer.capabilities, caps);
        assert_eq!(peer.addresses.len(), 1);
        assert_eq!(peer.addresses[0], "127.0.0.1:9000".parse().unwrap());
    }

    #[tokio::test]
    async fn test_discovery_service_creation_and_shutdown() {
        let config = DiscoveryConfig::default();
        let mut discovery = MdnsDiscovery::new(config).expect("should create MdnsDiscovery");
        assert!(discovery.directory().is_empty());

        discovery.start_discovery().expect("start discovery");
        discovery.shutdown().await.expect("shutdown discovery");
    }
}
