use crate::error::{DiscoveryError, Result};
use crate::peer::PeerDirectory;
use crate::service::{DiscoveryConfig, DiscoveryEvent, MdnsDiscovery};
use crate::udp::{UdpConfig, UdpDiscovery};
use bridge_core::{Capabilities, DeviceType, NodeId};
use std::net::IpAddr;
use tokio::sync::broadcast;
use tracing::info;

/// Modes of discovery operation supported by `UnifiedDiscovery`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedDiscoveryMode {
    /// Broadcast and discover using both mDNS and UDP beacons concurrently (recommended).
    Both,
    /// Rely exclusively on mDNS / DNS-SD.
    MdnsOnly,
    /// Rely exclusively on UDP broadcast beacons (for mDNS-restricted enterprise LANs).
    UdpOnly,
}

/// Configuration for unified multi-channel LAN peer discovery.
#[derive(Debug, Clone)]
pub struct UnifiedDiscoveryConfig {
    pub mode: UnifiedDiscoveryMode,
    pub mdns: DiscoveryConfig,
    pub udp: UdpConfig,
}

impl Default for UnifiedDiscoveryConfig {
    fn default() -> Self {
        Self {
            mode: UnifiedDiscoveryMode::Both,
            mdns: DiscoveryConfig::default(),
            udp: UdpConfig::default(),
        }
    }
}

/// Unified discovery engine orchestrating mDNS and UDP broadcast fallback over a shared peer directory.
pub struct UnifiedDiscovery {
    config: UnifiedDiscoveryConfig,
    directory: PeerDirectory,
    event_tx: broadcast::Sender<DiscoveryEvent>,
    mdns: Option<MdnsDiscovery>,
    udp: Option<UdpDiscovery>,
    is_running: bool,
}

impl std::fmt::Debug for UnifiedDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnifiedDiscovery")
            .field("mode", &self.config.mode)
            .field("directory", &self.directory)
            .field("is_running", &self.is_running)
            .finish_non_exhaustive()
    }
}

impl UnifiedDiscovery {
    /// Creates a new `UnifiedDiscovery` instance configured with the specified options.
    pub fn new(config: UnifiedDiscoveryConfig) -> Result<Self> {
        let directory = PeerDirectory::new();
        let (event_tx, _) = broadcast::channel(128);

        let mdns = match config.mode {
            UnifiedDiscoveryMode::Both | UnifiedDiscoveryMode::MdnsOnly => {
                Some(MdnsDiscovery::with_directory_and_events(
                    config.mdns.clone(),
                    directory.clone(),
                    event_tx.clone(),
                )?)
            }
            UnifiedDiscoveryMode::UdpOnly => None,
        };

        let udp = match config.mode {
            UnifiedDiscoveryMode::Both | UnifiedDiscoveryMode::UdpOnly => {
                Some(UdpDiscovery::with_directory_and_events(
                    config.udp.clone(),
                    directory.clone(),
                    event_tx.clone(),
                ))
            }
            UnifiedDiscoveryMode::MdnsOnly => None,
        };

        Ok(Self {
            config,
            directory,
            event_tx,
            mdns,
            udp,
            is_running: false,
        })
    }

    /// Accesses the unified shared peer directory.
    pub fn directory(&self) -> &PeerDirectory {
        &self.directory
    }

    /// Subscribes to peer discovery and departure events from all active channels.
    pub fn subscribe(&self) -> broadcast::Receiver<DiscoveryEvent> {
        self.event_tx.subscribe()
    }

    /// Returns the currently active operational mode.
    pub fn mode(&self) -> UnifiedDiscoveryMode {
        self.config.mode
    }

    /// Advertises this local node on all active discovery channels.
    pub async fn advertise(
        &mut self,
        node_id: NodeId,
        device_name: &str,
        device_type: DeviceType,
        port: u16,
        capabilities: Capabilities,
        ips: Option<Vec<IpAddr>>,
    ) -> Result<()> {
        if let Some(ref mdns) = self.mdns {
            mdns.advertise(node_id, device_name, device_type, port, capabilities, ips)
                .await?;
        }

        if let Some(ref udp) = self.udp {
            udp.advertise(node_id, device_name, device_type, port, capabilities)
                .await?;
        }

        info!(node_id = %node_id, mode = ?self.config.mode, "Advertised local node via UnifiedDiscovery");
        Ok(())
    }

    /// Stops advertising on all active discovery channels.
    pub async fn stop_advertising(&mut self) -> Result<()> {
        if let Some(ref mdns) = self.mdns {
            let _ = mdns.stop_advertising().await;
        }

        if let Some(ref udp) = self.udp {
            let _ = udp.stop_advertising().await;
        }

        info!("Stopped advertising on all UnifiedDiscovery channels");
        Ok(())
    }

    /// Starts passive listening and active peer discovery on all configured channels.
    pub fn start_discovery(&mut self) -> Result<()> {
        if self.is_running {
            return Err(DiscoveryError::AlreadyRunning);
        }

        if let Some(ref mut mdns) = self.mdns {
            mdns.start_discovery()?;
        }

        if let Some(ref mut udp) = self.udp {
            udp.start_discovery()?;
        }

        self.is_running = true;
        info!(mode = ?self.config.mode, "UnifiedDiscovery started");
        Ok(())
    }

    /// Shuts down all active discovery channels, unregisters services, and frees sockets.
    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(ref mut mdns) = self.mdns {
            let _ = mdns.shutdown().await;
        }

        if let Some(ref mut udp) = self.udp {
            let _ = udp.shutdown().await;
        }

        self.is_running = false;
        info!("UnifiedDiscovery shutdown complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_unified_discovery_modes() {
        // UdpOnly
        let udp_cfg = UdpConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            broadcast_targets: vec!["127.0.0.1:0".parse().unwrap()],
            ..UdpConfig::default()
        };
        let config_udp_only = UnifiedDiscoveryConfig {
            mode: UnifiedDiscoveryMode::UdpOnly,
            udp: udp_cfg.clone(),
            ..UnifiedDiscoveryConfig::default()
        };
        let mut unified_udp = UnifiedDiscovery::new(config_udp_only).expect("create UdpOnly");
        assert_eq!(unified_udp.mode(), UnifiedDiscoveryMode::UdpOnly);
        assert!(unified_udp.mdns.is_none());
        assert!(unified_udp.udp.is_some());
        unified_udp.start_discovery().expect("start UdpOnly");
        unified_udp.shutdown().await.expect("shutdown UdpOnly");

        // MdnsOnly
        let mdns_cfg = DiscoveryConfig {
            service_type: "_bridgeos-unified-test._tcp.local.".to_string(),
            ttl: Duration::from_secs(3),
            prune_interval: Duration::from_millis(500),
        };
        let config_mdns_only = UnifiedDiscoveryConfig {
            mode: UnifiedDiscoveryMode::MdnsOnly,
            mdns: mdns_cfg,
            ..UnifiedDiscoveryConfig::default()
        };
        let mut unified_mdns = UnifiedDiscovery::new(config_mdns_only).expect("create MdnsOnly");
        assert_eq!(unified_mdns.mode(), UnifiedDiscoveryMode::MdnsOnly);
        assert!(unified_mdns.mdns.is_some());
        assert!(unified_mdns.udp.is_none());
        unified_mdns.start_discovery().expect("start MdnsOnly");
        unified_mdns.shutdown().await.expect("shutdown MdnsOnly");
    }
}
