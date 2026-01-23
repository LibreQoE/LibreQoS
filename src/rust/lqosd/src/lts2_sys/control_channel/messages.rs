use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::lts2_sys::RemoteCommand;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RemoteInsightRequest {
    ShaperThroughput { seconds: i32 },
    ShaperPackets { seconds: i32 },
    ShaperPercent { seconds: i32 },
    ShaperFlows { seconds: i32 },
    ShaperRttHistogram { seconds: i32 },
    ShaperTopDownloaders { seconds: i32 },
    ShaperWorstRtt { seconds: i32 },
    ShaperWorstRxmit { seconds: i32 },
    ShaperTopFlows { seconds: i32 },
    ShaperRecentMedians,
    CakeStatsTotals { seconds: i32 },
}

pub const MAX_DECOMPRESSED_WS_MSG_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

#[derive(Serialize, Deserialize)]
pub enum WsMessage {
    // Messages FROM the Shaper
    Hello {
        magic: u32,
    },
    License {
        license: Uuid,
        node_id: String,
        node_name: String,
    },
    LicenseGrantRequest {
        public_key: Vec<u8>,
    },
    HeartbeatReply {
        insight_time: i64,
    },
    BeginIngest {
        unique_id: u64,
        n_chunks: u64,
    },
    IngestChunk {
        unique_id: u64,
        chunk: u64,
        n_chunks: u64,
        data: Vec<u8>,
    },
    EndIngest {
        unique_id: u64,
        n_chunks: u64,
    },
    RequestTopology {
        network_hash: i64,
        devices_hash: i64,
    },
    ApiReply {
        request_id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        data: Vec<u8>,
    },

    // Heartbeats (from Insight)
    Heartbeat {
        timestamp: i64,
    },

    // Replies TO the Shaper
    Welcome {
        valid: bool,
        license_state: i32,
        expiration_date: i64,
    },
    InsightPublicKey {
        public_key: Vec<u8>,
    },
    LicenseGrant {
        payload: Vec<u8>,
        signature: Vec<u8>,
    },
    YouMaySubmit {
        ingestion_id: u64,
    },
    RemoteCommands {
        commands: Vec<RemoteCommand>,
    },
    HereIsTopology {
        new_network_hash: i64,
        new_devices_hash: i64,
        chunk: u64,
        n_chunks: u64,
        data: Vec<u8>,
    },
    StartStreaming {
        request_id: u64,
        circuit_hash: i64,
    },
    StartShaperStreaming {
        request_id: u64,
    },
    // Request a one-shot shaper tree snapshot
    StartShaperTreeStreaming {
        request_id: u64,
    },
    StreamingCircuit {
        request_id: u64,
        circuit_hash: i64,
        data: Vec<u8>,
    },
    StreamingShaper {
        request_id: u64,
        bytes_down: u64,
        bytes_up: u64,
        shaped_bytes_down: u64,
        shaped_bytes_up: u64,
        packets_down: u64,
        packets_up: u64,
    },
    // Reply containing a one-shot shaper tree snapshot (CBOR encoded payload)
    StreamingShaperTree {
        request_id: u64,
        data: Vec<u8>,
    },
    HistoryQuery {
        request_id: u64,
        query: RemoteInsightRequest,
    },
    HistoryQueryResult {
        request_id: u64,
        tag: String,
        seconds: i32,
        data: Option<Vec<u8>>,
    },
    MakeApiRequest {
        request_id: u64,
        method: ApiRequestType,
        url_suffix: String,
        body: Option<String>,
    },
    // Chatbot (Ask Libby) streaming via Insight chatbot service
    // From Shaper -> Insight
    ChatbotStart {
        request_id: u64,
        browser_ts_ms: Option<i64>,
        browser_language: Option<String>,
    },
    ChatbotUserInput {
        request_id: u64,
        text: String,
    },
    ChatbotStop {
        request_id: u64,
    },
    // From Insight -> Shaper (streaming chunks or errors)
    ChatbotChunk {
        request_id: u64,
        data: Vec<u8>,
    },
    ChatbotError {
        request_id: u64,
        message: String,
    },
}

#[derive(Serialize, Deserialize)]
pub enum ApiRequestType {
    Get,
    Post,
    Delete,
}

impl WsMessage {
    pub fn to_bytes(&self) -> anyhow::Result<(usize, usize, Vec<u8>)> {
        let raw_bytes = serde_cbor::to_vec(self)?;
        let compressed_bytes = miniz_oxide::deflate::compress_to_vec(&raw_bytes, 10);
        Ok((raw_bytes.len(), compressed_bytes.len(), compressed_bytes))
    }

    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        let decompressed_bytes = miniz_oxide::inflate::decompress_to_vec_with_limit(
            bytes,
            MAX_DECOMPRESSED_WS_MSG_BYTES,
        )
        .map_err(|_e| anyhow!("Decompression error or size limit exceeded"))?;
        Ok(serde_cbor::from_slice(&decompressed_bytes)?)
    }
}
