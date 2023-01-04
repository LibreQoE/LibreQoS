use serde::{Deserialize, Serialize};
use crate::TcHandle;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IpStats {
    pub ip_address: String,
    pub bits_per_second: (u64, u64),
    pub packets_per_second: (u64, u64),
    pub median_tcp_rtt: f32,
    pub tc_handle: TcHandle,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IpMapping {
    pub ip_address: String,
    pub prefix_length: u32,
    pub tc_handle: TcHandle,
    pub cpu: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct XdpPpingResult {
    pub tc: String,
    pub avg: f32,
    pub min: f32,
    pub max: f32,
    pub median: f32,
    pub samples: u32,
}