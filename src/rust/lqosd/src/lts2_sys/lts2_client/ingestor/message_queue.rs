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

use std::net::TcpStream;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;
use lqos_config::load_config;
use crate::lts2_sys::lts2_client::ingestor::commands::IngestorCommand;
use crate::lts2_sys::lts2_client::{get_remote_host, remote_commands};
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

    pub(crate) fn send(&mut self) -> Result<()> {
        let config = load_config()?;
        if !config.long_term_stats.use_insight.unwrap_or(false) {
            self.clear();
            return Ok(());
        }

        let remote_host = get_remote_host();
        let target = format!("wss://{}:443/ingest/ws", remote_host);
        info!("Sending messages to {}", target);

        let Ok(stream) = TcpStream::connect(&format!("{}:443", remote_host)) else {
            warn!("Failed to connect to ingestion server");
            return Ok(());
        };

        let Ok(connector) = native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .build() else
        {
            warn!("Failed to create TLS connector");
            return Ok(());
        };

        let result = connector.connect(&format!("{}:443", remote_host), stream);
        if result.is_err() {
            warn!("Failed to connect TLS stream to ingestion server: {result:?}");
            return Ok(());
        }
        let Ok(tls_stream) = result else {
            warn!("Failed to connect TLS stream to ingestion server");
            return Ok(());
        };

        let result = tungstenite::client(target, tls_stream);
        if result.is_err() {
            warn!("Failed to connect to ingestion server. {result:?}");
            return Ok(());
        }
        let Ok((mut socket, _response)) = result else {
            warn!("Failed to connect to ingestion server");
            return Ok(());
        };

        // Send Hello
        let Ok((_, _, magic_to_send)) = (WsMessage::Hello { magic: 0x2763 }).to_bytes() else {
            warn!("Failed to serialize hello message");
            return Ok(());
        };
        if let Err(e) = socket.send(tungstenite::Message::Binary(magic_to_send)) {
            warn!("Failed to send hello message to server: {}", e);
            return Ok(());
        }

        // Wait for Hello Back
        let Ok(reply) = socket.read() else {
            warn!("Failed to receive hello response from server");
            return Ok(());
        };
        let Ok(reply) = WsMessage::from_bytes(&reply.into_data()) else {
            warn!("Failed to deserialize hello response from server");
            return Ok(());
        };
        match reply {
            WsMessage::Hello { magic } => {
                if magic != 0x3672 {
                    warn!("Received invalid magic number from server: {}", magic);
                    return Ok(());
                }
            }
            _ => {
                warn!("Received unexpected message from server");
                return Ok(());
            }
        }

        // Send License
        let (license_key, node_id, node_name) = {
            let lock = load_config().unwrap();
            (
                lock.long_term_stats.license_key.clone().unwrap_or("".to_string()),
                lock.node_id.clone(),
                lock.node_name.clone(),
            )
        };
        let Ok(license_uuid) = Uuid::parse_str(&license_key) else {
            warn!("Failed to parse license key");
            return Ok(());
        };
        let Ok((_, _, license_to_send)) = (WsMessage::License { license: license_uuid }).to_bytes() else {
            warn!("Failed to serialize license message");
            return Ok(());
        };
        if let Err(e) = socket.send(tungstenite::Message::Binary(license_to_send)) {
            warn!("Failed to send license message to server: {}", e);
            return Ok(());
        }

        // Wait for CanSubmit
        let Ok(reply) = socket.read() else {
            warn!("Failed to receive can submit response from server");
            return Ok(());
        };
        let Ok(reply) = WsMessage::from_bytes(&reply.into_data()) else {
            warn!("Failed to deserialize can submit response from server");
            return Ok(());
        };
        match reply {
            WsMessage::CanSubmit => {}
            _ => {
                warn!("Received unexpected message from server");
                return Ok(());
            }
        }

        // Build the submission packet
        let mut message = IngestSession {
            license_key: license_uuid,
            node_id: node_id.clone(),
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

        // Build the submission blob
        let Ok(raw_bytes) = serde_cbor::to_vec(&message) else {
            warn!("Failed to serialize data message");
            return Ok(());
        };
        let compressed_bytes = miniz_oxide::deflate::compress_to_vec(&raw_bytes, 10);

        // Divide into chunks. Chunk size is 60k
        const CHUNK_SIZE: usize = 60 * 1024;
        let message_chunks = compressed_bytes.chunks(CHUNK_SIZE);
        let n_chunks = message_chunks.len();

        // Submit the chunks
        for (i,chunk) in message_chunks.into_iter().enumerate() {
            let Ok((_, _, data_to_send)) = (WsMessage::DataDump { chunk: i+1, n_chunks, data: chunk.to_vec() }).to_bytes() else {
                warn!("Failed to serialize data message");
                return Ok(());
            };
            if let Err(e) = socket.send(tungstenite::Message::Binary(data_to_send)) {
                warn!("Failed to send data message to server: {}", e);
                return Ok(());
            }
        }

        // Remote Commands
        let Ok((_, _, request_remote_commands)) = (WsMessage::RequestRemoteCommands ).to_bytes() else {
            warn!("Failed to serialize request remote commands message");
            return Ok(());
        };
        if let Err(e) = socket.send(tungstenite::Message::Binary(request_remote_commands)) {
            warn!("Failed to send request remote commands message to server: {}", e);
            return Ok(());
        }

        // Wait for Remote Commands
        let Ok(reply) = socket.read() else {
            warn!("Failed to receive remote commands response from server");
            return Ok(());
        };
        let Ok(reply) = WsMessage::from_bytes(&reply.into_data()) else {
            warn!("Failed to deserialize remote commands response from server");
            return Ok(());
        };
        match reply {
            WsMessage::RemoteCommands { commands } => {
                remote_commands::enqueue(commands);
            }
            _ => {
                warn!("Received unexpected message from server");
                return Ok(());
            }
        }
        if let Err(e) = socket.close(None) {
            warn!("Failed to close connection to server: {}", e);
            return Ok(());
        }
        drop(socket);
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

#[derive(Serialize, Deserialize)]
enum WsMessage {
    // Request messages
    Hello { magic: u32 },
    License { license: Uuid },
    DataDump { chunk: usize, n_chunks: usize, data: Vec<u8> },
    RequestRemoteCommands,

    // Response messages
    CanSubmit,
    RemoteCommands { commands: Vec<RemoteCommand> },
}

impl WsMessage {
    fn to_bytes(&self) -> anyhow::Result<(usize, usize, Vec<u8>)> {
        let raw_bytes = serde_cbor::to_vec(self)?;
        let compressed_bytes = miniz_oxide::deflate::compress_to_vec(&raw_bytes, 10);
        Ok((raw_bytes.len(), compressed_bytes.len(), compressed_bytes))
    }

    fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        let decompressed_bytes = miniz_oxide::inflate::decompress_to_vec(&bytes)
            .map_err(|e| anyhow!("Decompression error: {e:?}"))?;
        Ok(serde_cbor::from_slice(&decompressed_bytes)?)
    }
}