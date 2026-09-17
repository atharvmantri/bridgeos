pub mod client;
pub mod node;

use anyhow::Result;
use bridge_core::DeviceType;
use bridge_discovery::{
    DiscoveryConfig, UdpConfig, UnifiedDiscovery, UnifiedDiscoveryConfig, UnifiedDiscoveryMode,
};
use bridge_identity::IdentityKey;
use clap::{Parser, Subcommand};
use node::NodeConfig;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Parser, Debug)]
#[command(
    name = "bridge-cli",
    about = "BridgeOS Developer CLI & Multi-Node Integration Test Harness",
    version,
    after_help = r#"MULTI-NODE TEST HARNESS EXAMPLES:

  Terminal 1 (Run Node A):
    bridge-cli node --name desktop-a --port 9801 --receive-dir ./received_a

  Terminal 2 (Run Node B):
    bridge-cli node --name desktop-b --port 9802 --receive-dir ./received_b

  Terminal 3 (Client Operations):
    # Discover active peers on LAN:
    bridge-cli discover --duration 3

    # Ping Node A:
    bridge-cli ping --peer 127.0.0.1:9801 --count 5

    # Stream a file to Node A:
    bridge-cli send-file --peer 127.0.0.1:9801 --file ./README.md

    # Display or generate Ed25519 identity:
    bridge-cli identity --generate
"#
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Log level filter (trace, debug, info, warn, error)
    #[arg(short, long, global = true, default_value = "info")]
    pub log_level: String,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Commands {
    /// Runs an active BridgeOS peer node with discovery, connection listening, and file reception
    Node {
        /// Human-readable device name
        #[arg(short, long, default_value = "BridgeOS-Node")]
        name: String,

        /// TCP listener port (0 for dynamic assignment)
        #[arg(short, long, default_value_t = 0)]
        port: u16,

        /// Device type (windows, android, linux, macos)
        #[arg(short, long, default_value = "windows")]
        device_type: String,

        /// Directory where incoming received files are written
        #[arg(short, long, default_value = "./received")]
        receive_dir: PathBuf,

        /// Persistent node data directory (identity key and trust.db)
        #[arg(long, default_value = "./data")]
        data_dir: PathBuf,

        /// UDP broadcast beacon port
        #[arg(long, default_value_t = 42424)]
        broadcast_port: u16,

        /// Disable mDNS advertising and browsing
        #[arg(long, default_value_t = false)]
        no_mdns: bool,

        /// Disable UDP broadcast beaconing
        #[arg(long, default_value_t = false)]
        no_udp: bool,
    },

    /// Performs passive and active LAN discovery to list discovered peers
    Discover {
        /// Discovery scan duration in seconds
        #[arg(short, long, default_value_t = 3)]
        duration: u64,

        /// UDP broadcast beacon port
        #[arg(long, default_value_t = 42424)]
        broadcast_port: u16,
    },

    /// Discovers active LAN peers and cross-references their trust authorization status
    Peers {
        /// Discovery scan duration in seconds
        #[arg(short, long, default_value_t = 3)]
        duration: u64,

        /// UDP broadcast beacon port
        #[arg(long, default_value_t = 42424)]
        broadcast_port: u16,

        /// Persistent node data directory containing trust.db
        #[arg(long, default_value = "./data")]
        data_dir: PathBuf,
    },

    /// Pings an authenticated peer to measure round-trip latency
    Ping {
        /// Target peer address (IP:PORT)
        #[arg(short, long)]
        peer: SocketAddr,

        /// Number of ping probes to send
        #[arg(short, long, default_value_t = 3)]
        count: usize,

        /// Client device name
        #[arg(short, long, default_value = "BridgeOS-PingClient")]
        name: String,
    },

    /// Streams an on-disk file to a remote peer using resumable chunked streaming
    SendFile {
        /// Target peer address (IP:PORT)
        #[arg(short, long)]
        peer: SocketAddr,

        /// Local file to stream
        #[arg(short, long)]
        file: PathBuf,

        /// Sender device name
        #[arg(short, long, default_value = "BridgeOS-Sender")]
        name: String,

        /// Persistent node data directory containing identity key
        #[arg(long, default_value = "./data")]
        data_dir: PathBuf,
    },

    /// Executes an explicit SAS pairing ceremony with a discovered peer
    Pair {
        /// Target peer address (IP:PORT)
        #[arg(short, long)]
        peer: SocketAddr,

        /// Initiator device name
        #[arg(short, long, default_value = "BridgeOS-PairClient")]
        name: String,

        /// Persistent node data directory containing identity and trust.db
        #[arg(long, default_value = "./data")]
        data_dir: PathBuf,
    },

    /// Manages the local persistent trust store
    Trust {
        #[command(subcommand)]
        action: TrustAction,

        /// Persistent node data directory containing trust.db
        #[arg(long, default_value = "./data")]
        data_dir: PathBuf,
    },

    /// Displays local Ed25519 node identity or generates a fresh keypair
    Identity {
        /// Generate and output a fresh ephemeral identity key
        #[arg(short, long, default_value_t = false)]
        generate: bool,
    },
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum TrustAction {
    /// Lists all trusted peers recorded in the trust database
    List,
    /// Revokes trust for a specific hexadecimal NodeId
    Revoke {
        /// Hexadecimal NodeId to revoke
        #[arg(short, long)]
        node_id: String,
    },
}

pub async fn run_discover(duration_secs: u64, broadcast_port: u16) -> Result<()> {
    println!(
        "Scanning local LAN for BridgeOS peers for {duration_secs} seconds (mDNS & UDP:{broadcast_port})...",
    );

    let udp_cfg = UdpConfig {
        broadcast_port,
        bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), broadcast_port),
        broadcast_targets: vec![SocketAddr::new(
            IpAddr::V4(Ipv4Addr::BROADCAST),
            broadcast_port,
        )],
        ..Default::default()
    };

    let unified_cfg = UnifiedDiscoveryConfig {
        mode: UnifiedDiscoveryMode::Both,
        mdns: DiscoveryConfig::default(),
        udp: udp_cfg,
    };

    let mut discovery = UnifiedDiscovery::new(unified_cfg)?;
    discovery.start_discovery()?;

    tokio::time::sleep(std::time::Duration::from_secs(duration_secs)).await;

    let peers = discovery.directory().list();
    if peers.is_empty() {
        println!("No BridgeOS peers discovered on the local network.");
    } else {
        println!("\nFound {} BridgeOS peer(s):", peers.len());
        for (i, peer) in peers.iter().enumerate() {
            println!("\n[{}] Name:         {}", i + 1, peer.device_name);
            println!("    Node ID:      {}", peer.node_id);
            println!("    Type:         {:?}", peer.device_type);
            println!("    Addresses:    {:?}", peer.addresses);
            println!("    Capabilities: {:?}", peer.capabilities);
        }
    }

    discovery.shutdown().await?;
    Ok(())
}

pub fn run_identity(generate: bool) -> Result<IdentityKey> {
    let key = if generate {
        println!("Generated fresh Ed25519 identity keypair:");
        IdentityKey::generate()
    } else {
        println!("Node Ed25519 identity:");
        IdentityKey::generate()
    };

    let pubkey = key.public_key();
    let node_id = key.node_id();

    println!("  Node ID:     {node_id}");
    println!("  Public Key:  {}", hex::encode(pubkey.to_bytes()));
    Ok(key)
}

pub async fn execute_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Node {
            name,
            port,
            device_type,
            receive_dir,
            data_dir,
            broadcast_port,
            no_mdns,
            no_udp,
        } => {
            let dev_type = DeviceType::from_str(&device_type).unwrap_or(DeviceType::Windows);
            let config = NodeConfig {
                name,
                port,
                device_type: dev_type,
                receive_dir,
                data_dir,
                broadcast_port,
                enable_mdns: !no_mdns,
                enable_udp: !no_udp,
            };
            node::run_node(config).await?;
        }

        Commands::Discover {
            duration,
            broadcast_port,
        } => {
            run_discover(duration, broadcast_port).await?;
        }

        Commands::Peers {
            duration,
            broadcast_port,
            data_dir,
        } => {
            client::run_peers(duration, broadcast_port, &data_dir).await?;
        }

        Commands::Ping { peer, count, name } => {
            client::run_ping(peer, count, &name).await?;
        }

        Commands::SendFile {
            peer,
            file,
            name,
            data_dir,
        } => {
            let key = bridge_identity::IdentityStorage::load_or_generate(
                data_dir.join("identity").join("secret.key"),
            )
            .ok();
            client::run_send_file(peer, &file, &name, key.as_ref()).await?;
        }

        Commands::Pair {
            peer,
            name,
            data_dir,
        } => {
            client::run_pair(peer, &name, &data_dir).await?;
        }

        Commands::Trust { action, data_dir } => match action {
            TrustAction::List => {
                client::run_trust_list(&data_dir)?;
            }
            TrustAction::Revoke { node_id } => {
                client::run_trust_revoke(&data_dir, &node_id)?;
            }
        },

        Commands::Identity { generate } => {
            let _ = run_identity(generate)?;
        }
    }

    Ok(())
}
