use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::lts2_sys::RemoteCommand;

pub const MAX_COMPRESSED_WS_MSG_BYTES: usize = 4 * 1024 * 1024; // 4 MiB
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
        circuit_hash: i64,
    },
    EndStreaming {
        circuit_hash: i64,
    },
    StreamingCircuit {
        circuit_hash: i64,
    },
    ChatConfiguration {
        default_model: Option<String>,
        allowed_models: Vec<String>,
    },
    ChatProxyRequest {
        request_id: Uuid,
        body: Value,
    },
    ChatProxyResponse {
        request_id: Uuid,
        success: bool,
        body: Option<Value>,
        error: Option<String>,
    },
    ChatProxyStream {
        request_id: Uuid,
        event: ChatStreamEvent,
    },
}

#[derive(Serialize, Deserialize)]
pub enum ChatStreamEvent {
    Begin,
    Delta { data: Value },
    End,
    Error { message: String },
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
