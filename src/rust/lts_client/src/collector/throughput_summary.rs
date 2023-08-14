//! Provides an interface for collecting data from the throughput
//! tracker in `lqosd` and submitting it into the long-term stats
//! system.
//! 
//! Note that ThroughputSummary should be boxed, to avoid copying

use std::net::IpAddr;

#[derive(Debug)]
pub struct ThroughputSummary {
    pub bits_per_second: (u64, u64),
    pub shaped_bits_per_second: (u64, u64),
    pub packets_per_second: (u64, u64),
    pub hosts: Vec<HostSummary>,
}

#[derive(Debug)]
pub struct HostSummary {
    pub ip: IpAddr,
    pub circuit_id: Option<String>,
    pub bits_per_second: (u64, u64),
    pub median_rtt: f32,
}