use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsSummary {
    pub min: (u64, u64),
    pub max: (u64, u64),
    pub avg: (u64, u64),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsRttSummary {
    pub min: u32,
    pub max: u32,
    pub avg: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsTotals {
    pub packets: StatsSummary,
    pub bits: StatsSummary,
    pub shaped_bits: StatsSummary,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsHost {
    pub circuit_id: String,
    pub ip_address: String,
    pub bits: StatsSummary,
    pub rtt: StatsRttSummary,
}