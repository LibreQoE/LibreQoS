mod circuit_cake_drops;
mod circuit_cake_marks;
mod circuit_retransmits;
mod circuit_rtt;
mod circuit_throughput;
mod general;
mod site_cake_drops;
mod site_cake_marks;
mod site_retransmits;
mod site_rtt;
mod site_throughput;

use crate::lts2_sys::lts2_client::ingestor::commands::IngestorCommand;
use crate::lts2_sys::shared_types::{
    CircuitCakeDrops, CircuitCakeMarks, CircuitRetransmits, CircuitRtt, CircuitThroughput,
    IngestSession, SiteCakeDrops, SiteCakeMarks, SiteRetransmits, SiteRtt, SiteThroughput,
};
use anyhow::Result;
use lqos_config::load_config;
use tracing::{debug, warn};
use uuid::Uuid;

/// Provides holders for messages that have been received from the ingestor,
/// and not yet submitted to the LTS2 server. It divides many message types by
/// the type, to maximize batching.
#[derive(Clone)]
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
        self.general_queue.is_empty()
            && self.circuit_throughput.is_empty()
            && self.circuit_retransmits.is_empty()
            && self.circuit_rtt.is_empty()
            && self.circuit_cake_drops.is_empty()
            && self.circuit_cake_marks.is_empty()
            && self.site_throughput.is_empty()
            && self.site_retransmits.is_empty()
            && self.site_cake_drops.is_empty()
            && self.site_cake_marks.is_empty()
            && self.site_rtt.is_empty()
    }

    pub(crate) fn ingest(&mut self, command: IngestorCommand) {
        match command {
            IngestorCommand::IngestBatchComplete { .. } => {
                tracing::error!("IngestBatchComplete Should Never Reach Here");
            }
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

    pub(crate) fn build_chunks(&mut self) -> Result<Vec<Vec<u8>>> {
        // Gather the license info for bundling
        let (license_key, node_id, node_name) = {
            let Ok(lock) = load_config() else {
                warn!("Failed to load config");
                return Ok(vec![]);
            };
            (
                lock.long_term_stats
                    .license_key
                    .clone()
                    .unwrap_or("".to_string()),
                lock.node_id.clone(),
                lock.node_name.clone(),
            )
        };
        let Ok(license_uuid) = Uuid::parse_str(&license_key.replace("-", "")) else {
            warn!("Failed to parse license key");
            return Ok(vec![]);
        };

        // Build the message
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
            return Ok(vec![]);
        };
        let compressed_bytes = miniz_oxide::deflate::compress_to_vec(&raw_bytes, 10);

        // Divide into chunks. Chunk size is 60k
        const CHUNK_SIZE: usize = 60 * 1024;
        let message_chunks = compressed_bytes.chunks(CHUNK_SIZE);
        let n_chunks = message_chunks.len();
        let chunks = message_chunks
            .map(|chunk| chunk.to_vec())
            .collect::<Vec<_>>();
        debug!("Submitting {} chunks of data", n_chunks);
        Ok(chunks)
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
