// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

use super::QueueStoreTransit;
use crate::{
    Circuit, IpMapping, IpStats, XdpPpingResult,
    ip_stats::{FlowbeeSummaryData, PacketHeader},
};
use allocative::Allocative;
use lqos_utils::{temporal_heatmap::HeatmapBlocks, units::DownUpOrder};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// An urgent issue to be displayed prominently in the UI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct UrgentIssue {
    /// Unique identifier
    pub id: u64,
    /// Unix timestamp (seconds)
    pub ts: u64,
    /// Source component
    pub source: crate::bus::request::UrgentSource,
    /// Severity level
    pub severity: crate::bus::request::UrgentSeverity,
    /// Machine-readable code (e.g., TC_U16_OVERFLOW)
    pub code: String,
    /// Human-readable message
    pub message: String,
    /// Optional JSON context
    pub context: Option<String>,
    /// Optional dedupe key
    pub dedupe_key: Option<String>,
}
/// Serializable snapshot of BakeryStats for bus transmission
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct BakeryStatsSnapshot {
    /// The number of active circuits in the bakery
    pub active_circuits: u64,
}

/// Circuit-level TemporalHeatmap data for the executive summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct CircuitHeatmapData {
    /// Circuit hash identifier from ShapedDevices.csv.
    pub circuit_hash: i64,
    /// Circuit ID string.
    pub circuit_id: String,
    /// Circuit name string.
    pub circuit_name: String,
    /// Heatmap blocks for the circuit.
    pub blocks: HeatmapBlocks,
}

/// A `BusResponse` object represents a single
/// reply generated from a `BusRequest`, and batched
/// inside a `BusReply`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Allocative)]
pub enum BusResponse {
    /// Yes, we're alive
    Ack,

    /// An operation failed, with the enclosed error message.
    Fail(String),

    /// We aren't ready to process your call, please stay on the line
    /// and try later.
    NotReadyYet,

    /// Current throughput for the overall system.
    CurrentThroughput {
        /// In bps
        bits_per_second: DownUpOrder<u64>,

        /// In pps
        packets_per_second: DownUpOrder<u64>,

        /// PPS TCP only
        tcp_packets_per_second: DownUpOrder<u64>,

        /// PPS UDP only
        udp_packets_per_second: DownUpOrder<u64>,

        /// PPS ICMP only
        icmp_packets_per_second: DownUpOrder<u64>,

        /// How much of the response has been subject to the shaper?
        shaped_bits_per_second: DownUpOrder<u64>,
    },

    /// Provides a list of ALL mapped hosts traffic counters,
    /// listing the IP Address and upload/download in a tuple.
    HostCounters(Vec<(IpAddr, DownUpOrder<u64>)>),

    /// Provides the Top N downloaders IP stats.
    TopDownloaders(Vec<IpStats>),

    /// Provides the Top N uploaders IP stats.
    TopUploaders(Vec<IpStats>),

    /// Provides circuit-level heatmaps.
    CircuitHeatmaps(Vec<CircuitHeatmapData>),

    /// Provides the worst N RTT scores, sorted in descending order.
    WorstRtt(Vec<IpStats>),

    /// Provides the worst N Retransmit scores, sorted in descending order.
    WorstRetransmits(Vec<IpStats>),

    /// Provides the best N RTT scores, sorted in descending order.
    BestRtt(Vec<IpStats>),

    /// List all IP/TC mappings.
    MappedIps(Vec<IpMapping>),

    /// Return the data required for compatability with the `xdp_pping`
    /// program.
    XdpPping(Vec<XdpPpingResult>),

    /// Return the data required to render the RTT histogram on the
    /// local web GUI.
    RttHistogram(Vec<u32>),

    /// A tuple of (mapped)(unknown) host counts.
    HostCounts((u32, u32)),

    /// A list of all unmapped IP addresses that have been detected.
    AllUnknownIps(Vec<IpStats>),

    /// The results of reloading LibreQoS.
    ReloadLibreQoS(String),

    /// Validation results for checking ShapedDevices.csv
    ShapedDevicesValidation(String),

    /// A string containing a JSON dump of a queue stats. Analagos to
    /// the response from `tc show qdisc`.
    RawQueueData(Option<Box<QueueStoreTransit>>),

    /// Results from network map queries
    NetworkMap(Vec<(usize, lqos_config::NetworkJsonTransport)>),

    /// Named nodes from network.json
    NodeNames(Vec<(usize, String)>),

    /// Circuit data
    CircuitData(Vec<Circuit>),

    /// Statistics from lqosd
    LqosdStats {
        /// Number of bus requests handled
        bus_requests: u64,
        /// Us to poll hosts
        time_to_poll_hosts: u64,
        /// High traffic watermark
        high_watermark: DownUpOrder<u64>,
        /// Number of flows tracked
        tracked_flows: u64,
        /// RTT events per second
        rtt_events_per_second: u64,
    },

    /// The index of the new packet collection session
    PacketCollectionSession {
        /// The identifier of the capture session
        session_id: usize,
        /// Number of seconds for which data will be captured
        countdown: usize,
    },

    /// Packet header dump
    PacketDump(Option<Vec<PacketHeader>>),

    /// Pcap format dump
    PcapDump(Option<String>),

    /// All Active Flows (Not Recommended - Debug Use)
    AllActiveFlows(Vec<FlowbeeSummaryData>),

    /// Count active flows
    CountActiveFlows(u64),

    /// Top Flopws
    TopFlows(Vec<FlowbeeSummaryData>),

    /// Flows by IP
    FlowsByIp(Vec<FlowbeeSummaryData>),

    /// Current endpoints by country
    CurrentEndpointsByCountry(Vec<(String, DownUpOrder<u64>, [f32; 2], String)>),

    /// Current Lat/Lon of endpoints
    CurrentLatLon(Vec<(f64, f64, String, u64, f32)>),

    /// Duration of flows
    FlowDuration(Vec<(usize, u64)>),

    /// Summary of Ether Protocol
    EtherProtocols {
        /// Number of IPv4 Bytes
        v4_bytes: DownUpOrder<u64>,
        /// Number of IPv6 Bytes
        v6_bytes: DownUpOrder<u64>,
        /// Number of IPv4 Packets
        v4_packets: DownUpOrder<u64>,
        /// Number of IPv6 Packets
        v6_packets: DownUpOrder<u64>,
        /// Number of IPv4 Flows
        v4_rtt: DownUpOrder<u64>,
        /// Number of IPv6 Flows
        v6_rtt: DownUpOrder<u64>,
    },

    /// Summary of IP Protocols
    IpProtocols(Vec<(String, DownUpOrder<u64>)>),

    /// Stormguard statistics
    StormguardStats(Vec<(String, u64, u64)>),

    /// Bakery statistics
    BakeryActiveCircuits(usize),

    /// Scheduler status
    SchedulerStatus {
        /// Is the scheduler running
        running: bool,
        /// Any error message from integrations
        error: Option<String>,
    },

    /// List of urgent issues
    UrgentIssues(Vec<UrgentIssue>),

    /// Is Insight Enabled?
    InsightStatus(bool),
}
