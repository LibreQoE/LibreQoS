mod general;
mod circuit_throughput;
mod circuit_retransmits;
mod circuit_rtt;
mod circuit_cake_drops;
mod circuit_cake_marks;
mod site_throughput;
mod site_retransmits;
mod site_rtt;
mod site_cake_drops;
mod site_cake_marks;

use std::net::{SocketAddr, TcpStream};
use anyhow::Result;
use nacl_blob::KeyStore;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;
use lqos_config::load_config;
use crate::lts2_sys::lts2_client::ingestor::commands::IngestorCommand;
use crate::lts2_sys::lts2_client::{get_remote_host, nacl_blob, remote_commands};
use crate::lts2_sys::RemoteCommand;
use crate::lts2_sys::shared_types::{CircuitCakeDrops, CircuitCakeMarks, CircuitRetransmits, CircuitRtt, CircuitThroughput, IngestSession, SiteCakeDrops, SiteCakeMarks, SiteRetransmits, SiteRtt, SiteThroughput};

/// Provides holders for messages that have been received from the ingestor,
/// and not yet submitted to the LTS2 server. It divides many message types by
/// the type, to maximize batching.
pub(crate) struct MessageQueue {
    /// All messages of type `IngestorCommand::General` that have been received,
    /// that haven't been categorized for batching.
    general_queue: Vec<IngestorCommand>,
    circuit_throughput: Vec<CircuitThroughput>,
    circuit_retransmits: Vec<CircuitRetransmits>,
    circuit_rtt: Vec<CircuitRtt>,
    circuit_cake_drops: Vec<CircuitCakeDrops>,
    circuit_cake_marks: Vec<CircuitCakeMarks>,
    site_throughput: Vec<SiteThroughput>,
    site_retransmits: Vec<SiteRetransmits>,
    site_cake_drops: Vec<SiteCakeDrops>,
    site_cake_marks: Vec<SiteCakeMarks>,
    site_rtt: Vec<SiteRtt>,
}

impl MessageQueue {
    pub(crate) fn new() -> Self {
        Self {
            general_queue: Vec::new(),
            circuit_throughput: Vec::new(),
            circuit_retransmits: Vec::new(),
            circuit_rtt: Vec::new(),
            circuit_cake_drops: Vec::new(),
            circuit_cake_marks: Vec::new(),
            site_throughput: Vec::new(),
            site_retransmits: Vec::new(),
            site_cake_drops: Vec::new(),
            site_cake_marks: Vec::new(),
            site_rtt: Vec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.general_queue.is_empty() && self.circuit_throughput.is_empty() && self.circuit_retransmits.is_empty()
            && self.circuit_rtt.is_empty() && self.circuit_cake_drops.is_empty() && self.circuit_cake_marks.is_empty()
            && self.site_throughput.is_empty() && self.site_retransmits.is_empty() && self.site_cake_drops.is_empty()
            && self.site_cake_marks.is_empty() && self.site_rtt.is_empty()
    }

    pub(crate) fn ingest(&mut self, command: IngestorCommand) {
        match command {
            IngestorCommand::CircuitThroughputBatch(batch) => {
                self.circuit_throughput.extend(batch);
            }
            IngestorCommand::CircuitRetransmitsBatch(batch) => {
                self.circuit_retransmits.extend(batch);
            }
            IngestorCommand::CircuitRttBatch(batch) => {
                self.circuit_rtt.extend(batch);
            }
            IngestorCommand::CircuitCakeDropsBatch(batch) => {
                self.circuit_cake_drops.extend(batch);
            }
            IngestorCommand::CircuitCakeMarksBatch(batch) => {
                self.circuit_cake_marks.extend(batch);
            }
            IngestorCommand::SiteThroughputBatch(batch) => {
                self.site_throughput.extend(batch);
            }
            IngestorCommand::SiteRetransmitsBatch(batch) => {
                self.site_retransmits.extend(batch);
            }
            IngestorCommand::SiteCakeDropsBatch(batch) => {
                self.site_cake_drops.extend(batch);
            }
            IngestorCommand::SiteCakeMarksBatch(batch) => {
                self.site_cake_marks.extend(batch);
            }
            IngestorCommand::SiteRttBatch(batch) => {
                self.site_rtt.extend(batch);
            }
            _ => self.general_queue.push(command),
        }
    }

    pub(crate) fn send(&mut self, keys: Arc<KeyStore>) -> Result<()> {
        let config = load_config()?;
        if config.long_term_stats.use_insight.unwrap_or(false) {
            self.clear();
            return Ok(());
        }

        use std::net::ToSocketAddrs;
        let remote_host = get_remote_host();
        let target = &format!("{}:9121", remote_host);
        let mut to = target.to_socket_addrs()?;
        let to = to.next().ok_or_else(|| anyhow::anyhow!("Failed to resolve remote host"))?;
        if let Ok(mut socket) = TcpStream::connect_timeout(&to, Duration::from_secs(5)) {
            socket.set_nodelay(true)?;
            if let Err(e) = nacl_blob::transmit_hello(&keys, 0x2763, 1, &mut socket) {
                println!("Failed to send hello to ingestion server. {e:?}");
                return Ok(());
            }

            if let Ok((server_hello, _)) = nacl_blob::receive_hello(&mut socket) {
                //println!("Received hello from server");
                // Build the ingestion message
                let (license_key, node_id, node_name) = {
                    let lock = load_config().unwrap();
                    (
                        lock.long_term_stats.license_key.clone().unwrap_or("".to_string()),
                        lock.node_id.clone(),
                        lock.node_name.clone(),
                    )
                };
                if let Ok(key) = Uuid::parse_str(&license_key) {
                    let mut message = IngestSession {
                        license_key: key,
                        node_id,
                        node_name,
                        ..Default::default()
                    };
                    general::add_general(&mut message, &mut self.general_queue);
                    circuit_throughput::add_circuit_throughput(&mut message, &mut self.circuit_throughput);
                    circuit_retransmits::add_circuit_retransmits(&mut message, &mut self.circuit_retransmits);
                    circuit_rtt::add_circuit_rtt(&mut message, &mut self.circuit_rtt);
                    circuit_cake_drops::add_circuit_cake_drops(&mut message, &mut self.circuit_cake_drops);
                    circuit_cake_marks::add_circuit_cake_marks(&mut message, &mut self.circuit_cake_marks);
                    site_cake_drops::add_site_cake_drops(&mut message, &mut self.site_cake_drops);
                    site_cake_marks::add_site_cake_marks(&mut message, &mut self.site_cake_marks);
                    site_retransmits::add_site_retransmits(&mut message, &mut self.site_retransmits);
                    site_rtt::add_site_rtt(&mut message, &mut self.site_rtt);
                    site_throughput::add_site_throughput(&mut message, &mut self.site_throughput);
                    if let Err(e) = nacl_blob::transmit_payload(&keys, &server_hello.public_key, &message, &mut socket) {
                        println!("Failed to send ingestion message to server. {e:?}");
                    }

                    // Receive any commands from the remote server
                    match nacl_blob::receive_payload::<Vec<RemoteCommand>>(&keys, &server_hello.public_key, &mut socket) {
                        Ok((commands, _size)) => {
                            remote_commands::enqueue(commands);
                        }
                        Err(e) => {
                            println!("Failed to receive commands from server. {e:?}");
                        }
                    }
                }
            } else {
                println!("Failed to receive hello from server");
            }
        } else {
            println!("Failed to connect to ingestion server");
        }
        println!("Finished sending messages to {}", remote_host);
        Ok(())
    }

    pub(crate) fn clear(&mut self) {
        self.general_queue.clear();
        self.circuit_throughput.clear();
        self.circuit_retransmits.clear();
        self.circuit_rtt.clear();
        self.circuit_cake_drops.clear();
        self.circuit_cake_marks.clear();
        self.site_throughput.clear();
        self.site_retransmits.clear();
        self.site_cake_drops.clear();
        self.site_cake_marks.clear();
        self.site_rtt.clear();
    }
}