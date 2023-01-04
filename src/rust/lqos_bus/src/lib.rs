mod ip_stats;
use anyhow::Result;
pub use ip_stats::{IpMapping, IpStats, XdpPpingResult};
use serde::{Deserialize, Serialize};
mod tc_handle;
pub use tc_handle::TcHandle;

pub const BUS_BIND_ADDRESS: &str = "127.0.0.1:9999";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BusSession {
    pub auth_cookie: u32,
    pub requests: Vec<BusRequest>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum BusRequest {
    Ping, // A generic "is it alive" test
    GetCurrentThroughput,
    GetTopNDownloaders(u32),
    GetWorstRtt(u32),
    MapIpToFlow {
        ip_address: String,
        tc_handle: TcHandle,
        cpu: u32,
        upload: bool,
    },
    DelIpFlow {
        ip_address: String,
        upload: bool,
    },
    ClearIpFlow,
    ListIpFlow,
    XdpPping,
    RttHistogram,
    HostCounts,
    AllUnknownIps,
    ReloadLibreQoS,
    GetRawQueueData(String), // The string is the circuit ID
    #[cfg(feature = "equinix_tests")]
    RequestLqosEquinixTest, // TODO: Feature flag this so it doesn't go into production
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BusReply {
    pub auth_cookie: u32,
    pub responses: Vec<BusResponse>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum BusResponse {
    Ack,          // Yes, we're alive
    Fail(String), // The operation failed
    CurrentThroughput {
        bits_per_second: (u64, u64),
        packets_per_second: (u64, u64),
        shaped_bits_per_second: (u64, u64),
    },
    TopDownloaders(Vec<IpStats>),
    WorstRtt(Vec<IpStats>),
    MappedIps(Vec<IpMapping>),
    XdpPping(Vec<XdpPpingResult>),
    RttHistogram(Vec<u32>),
    HostCounts((u32, u32)),
    AllUnknownIps(Vec<IpStats>),
    ReloadLibreQoS(String),
    RawQueueData(String),
}

pub fn encode_request(request: &BusSession) -> Result<Vec<u8>> {
    Ok(bincode::serialize(request)?)
}

pub fn decode_request(bytes: &[u8]) -> Result<BusSession> {
    Ok(bincode::deserialize(&bytes)?)
}

pub fn encode_response(request: &BusReply) -> Result<Vec<u8>> {
    Ok(bincode::serialize(request)?)
}

pub fn decode_response(bytes: &[u8]) -> Result<BusReply> {
    Ok(bincode::deserialize(&bytes)?)
}

pub fn cookie_value() -> u32 {
    1234
}
