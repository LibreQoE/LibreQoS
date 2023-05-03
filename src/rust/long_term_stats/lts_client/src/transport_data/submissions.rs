//! Holds data-types to be submitted as part of long-term stats
//! collection.

use lqos_config::ShapedDevice;
use serde::{Serialize, Deserialize};

/// Type that provides a minimum, maximum and average value
/// for a given statistic within the associated time period.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsSummary {
    /// Minimum value
    pub min: (u64, u64),
    /// Maximum value
    pub max: (u64, u64),
    /// Average value
    pub avg: (u64, u64),
}

/// Type that provides a minimum, maximum and average value
/// for a given RTT value within the associated time period.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsRttSummary {
    /// Minimum value
    pub min: u32,
    /// Maximum value
    pub max: u32,
    /// Average value
    pub avg: u32,
}

/// Type that holds total traffic statistics for a given time period
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsTotals {
    /// Total number of packets
    pub packets: StatsSummary,
    /// Total number of bits
    pub bits: StatsSummary,
    /// Total number of shaped bits
    pub shaped_bits: StatsSummary,
}

/// Type that holds per-host statistics for a given stats collation
/// period.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsHost {
    /// Host circuit_id as it appears in ShapedDevices.csv
    pub circuit_id: Option<String>,
    /// Host's IP address
    pub ip_address: String,
    /// Host's traffic statistics
    pub bits: StatsSummary,
    /// Host's RTT statistics
    pub rtt: StatsRttSummary,
}

/// Node inside a traffic summary tree
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsTreeNode {
    /// Index in the tree vector
    pub index: usize,
    /// Name (from network.json)
    pub name: String,
    /// Maximum allowed throughput (from network.json)
    pub max_throughput: (u32, u32),
    /// Current throughput (from network.json)
    pub current_throughput: StatsSummary,
    /// RTT summaries
    pub rtt: StatsRttSummary,
    /// Indices of parents in the tree
    pub parents: Vec<usize>,
    /// Index of immediate parent in the tree
    pub immediate_parent: Option<usize>,
    /// Node Type
    pub node_type: Option<String>,
}

/// Collation of all stats for a given time period
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsSubmission {
    /// Timestamp of the collation (UNIX time)
    pub timestamp: u64,
    /// Total traffic statistics
    pub totals: Option<StatsTotals>,
    /// Per-host statistics
    pub hosts: Option<Vec<StatsHost>>,
    /// Tree of traffic summaries
    pub tree: Option<Vec<StatsTreeNode>>,
    /// CPU utiliation on the shaper
    pub cpu_usage: Vec<u32>,
    /// RAM utilization on the shaper
    pub ram_percent: u32,
}

/// Submission to the `lts_node` process
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum LtsCommand {
    Submit(Box<StatsSubmission>),
    Devices(Vec<ShapedDevice>),
}