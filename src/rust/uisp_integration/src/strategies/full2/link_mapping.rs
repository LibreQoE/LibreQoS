use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub enum LinkMapping {
    Ethernet { speed_mbps: u64 },
    DevicePair { device_a: String, device_b: String, speed_mbps: u64, }
}

impl LinkMapping {
    pub fn ethernet(speed_mbps: u64) -> Self {
        LinkMapping::Ethernet { speed_mbps }
    }

    pub fn capacity_mbps(&self) -> u64 {
        match self {
            LinkMapping::Ethernet { speed_mbps } => *speed_mbps,
            LinkMapping::DevicePair { speed_mbps, .. } => *speed_mbps,
        }
    }
}