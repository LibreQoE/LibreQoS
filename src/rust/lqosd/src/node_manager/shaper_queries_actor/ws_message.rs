use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum WsMessage {
    // Requests
    IdentifyYourself,
    InvalidToken,
    TokenAccepted,
    ShaperThroughput { seconds: i32 },
    ShaperPackets { seconds: i32 },
    ShaperPercent { seconds: i32 },
    ShaperFlows { seconds: i32 },
    ShaperRttHistogram { seconds: i32 },

    // Responses
    Hello { license_key: String, node_id: String },
    QueryResult { tag: String, seconds: i32, data: Vec<u8> },
    Tick,
}

impl WsMessage {
    pub fn to_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let raw_bytes = serde_cbor::to_vec(self)?;
        Ok(raw_bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        Ok(serde_cbor::from_slice(&bytes)?)
    }
}