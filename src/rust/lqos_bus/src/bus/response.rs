// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

use super::QueueStoreTransit;
use crate::{
    Circuit, IpMapping, IpStats, XdpPpingResult,
    ip_stats::{FlowbeeSummaryData, PacketHeader},
};
use allocative::Allocative;
use lqos_utils::{HeatmapBlocks, qoq_heatmap::QoqHeatmapBlocks, units::DownUpOrder};
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
    /// QoO/QoQ score heatmap blocks (optional; UI-only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qoq_blocks: Option<QoqHeatmapBlocks>,
}

/// Site-level TemporalHeatmap data for the executive summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct SiteHeatmapData {
    /// Site name from network.json.
    pub site_name: String,
    /// Optional node type from network.json (e.g., Site/AP).
    #[serde(default)]
    pub node_type: Option<String>,
    /// Depth of the site within the network tree (root is 0).
    #[serde(default)]
    pub depth: usize,
    /// Heatmap blocks for the site.
    pub blocks: HeatmapBlocks,
    /// QoO/QoQ score heatmap blocks (optional; UI-only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qoq_blocks: Option<QoqHeatmapBlocks>,
}

/// ASN-level TemporalHeatmap data for the executive summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct AsnHeatmapData {
    /// ASN number.
    pub asn: u32,
    /// ASN descriptive name (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asn_name: Option<String>,
    /// Heatmap blocks for the ASN.
    pub blocks: HeatmapBlocks,
}

/// Metrics for the Executive Summary header cards.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative, Default)]
pub struct ExecutiveSummaryHeader {
    /// Total number of unique circuits from SHAPED_DEVICES.
    pub circuit_count: u64,
    /// Total number of shaped devices.
    pub device_count: u64,
    /// Total number of sites in the site tree.
    pub site_count: u64,
    /// Number of mapped IPs (shaped).
    pub mapped_ip_count: u64,
    /// Number of unmapped IPs (unknown).
    pub unmapped_ip_count: u64,
    /// Number of HTB queues being tracked.
    pub htb_queue_count: u64,
    /// Number of CAKE queues being tracked.
    pub cake_queue_count: u64,
    /// Whether Insight is connected.
    pub insight_connected: bool,
}

/// Debug snapshot of StormGuard evaluation for one direction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct StormguardDebugDirection {
    /// Current queue rate (Mbps)
    pub queue_mbps: u64,
    /// Minimum allowed (Mbps)
    pub min_mbps: u64,
    /// Maximum allowed (Mbps)
    pub max_mbps: u64,
    /// Latest measured throughput (Mbps)
    pub throughput_mbps: f64,
    /// Moving-average throughput (Mbps)
    pub throughput_ma_mbps: Option<f64>,
    /// Latest retransmit fraction (0-1)
    pub retrans: Option<f64>,
    /// Moving-average retransmit fraction (0-1)
    pub retrans_ma: Option<f64>,
    /// Latest RTT sample (as reported)
    pub rtt: Option<f64>,
    /// Moving-average RTT
    pub rtt_ma: Option<f64>,
    /// State (Warmup/Running/Cooldown)
    pub state: String,
    /// Seconds remaining in cooldown, if applicable
    pub cooldown_remaining_secs: Option<f32>,
    /// Saturation level vs current queue
    pub saturation_current: String,
    /// Saturation level vs max plan
    pub saturation_max: String,
    /// Whether StormGuard can increase this direction
    pub can_increase: bool,
    /// Whether StormGuard can decrease this direction
    pub can_decrease: bool,
}

/// Debug snapshot of StormGuard evaluation for a site
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct StormguardDebugEntry {
    /// Site name
    pub site: String,
    /// Download direction debug data
    pub download: StormguardDebugDirection,
    /// Upload direction debug data
    pub upload: StormguardDebugDirection,
}

/// Device counts (shaped devices and unknown IPs)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub struct DeviceCounts {
    /// Number of shaped devices
    pub shaped_devices: usize,
    /// Number of unknown IPs
    pub unknown_ips: usize,
}

/// Circuit counts (active vs configured)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub struct CircuitCount {
    /// Active circuit count
    pub count: usize,
    /// Configured circuit count
    pub configured_count: usize,
}

/// Flow map point for recent flow endpoints
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct FlowMapPoint {
    /// Latitude of the endpoint
    pub lat: f64,
    /// Longitude of the endpoint
    pub lon: f64,
    /// Country label
    pub country: String,
    /// Bytes sent (download direction)
    pub bytes_sent: u64,
    /// RTT sample in nanoseconds
    pub rtt_nanos: f32,
}

/// ASN list entry with recent flow counts
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub struct AsnListEntry {
    /// Flow count for this ASN
    pub count: usize,
    /// ASN number
    pub asn: u32,
    /// ASN name
    pub name: String,
}

/// Country list entry with recent flow counts
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub struct CountryListEntry {
    /// Flow count for this country
    pub count: usize,
    /// Country name
    pub name: String,
    /// Country ISO code
    pub iso_code: String,
}

/// Protocol list entry with recent flow counts
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub struct ProtocolListEntry {
    /// Flow count for this protocol
    pub count: usize,
    /// Protocol name
    pub protocol: String,
}

/// Flow timeline entry for flow explorer
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct FlowTimelineEntry {
    /// Flow start time (unix seconds)
    pub start: u64,
    /// Flow end time (unix seconds)
    pub end: u64,
    /// Flow duration in nanoseconds
    pub duration_nanos: u64,
    /// Optional throughput series
    pub throughput: Vec<DownUpOrder<u64>>,
    /// TCP retransmit counts
    pub tcp_retransmits: DownUpOrder<u16>,
    /// RTT samples in nanoseconds (down/up)
    pub rtt_nanos: [u64; 2],
    /// Retransmit timestamps for download (unix seconds)
    pub retransmit_times_down: Vec<u64>,
    /// Retransmit timestamps for upload (unix seconds)
    pub retransmit_times_up: Vec<u64>,
    /// Total bytes sent
    pub total_bytes: DownUpOrder<u64>,
    /// Protocol name
    pub protocol: String,
    /// Circuit ID
    pub circuit_id: String,
    /// Circuit name
    pub circuit_name: String,
    /// Remote IP address
    pub remote_ip: String,
}

/// Scheduler details response
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub struct SchedulerDetails {
    /// Whether the scheduler is available
    pub available: bool,
    /// Optional error message
    pub error: Option<String>,
    /// Human-readable diagnostics
    pub details: String,
}

/// Queue statistics totals (marks/drops)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub struct QueueStatsTotal {
    /// Total marks (down/up)
    pub marks: DownUpOrder<u64>,
    /// Total drops (down/up)
    pub drops: DownUpOrder<u64>,
}

/// Circuit capacity utilization row
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct CircuitCapacityRow {
    /// Circuit ID
    pub circuit_id: String,
    /// Circuit name
    pub circuit_name: String,
    /// Capacity ratios [down, up]
    pub capacity: [f64; 2],
    /// Median RTT
    pub median_rtt: f32,
}

/// Node capacity utilization row
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Allocative)]
pub struct NodeCapacity {
    /// Node ID
    pub id: usize,
    /// Node name
    pub name: String,
    /// Current down Mbps
    pub down: f64,
    /// Current up Mbps
    pub up: f64,
    /// Max down Mbps
    pub max_down: f64,
    /// Max up Mbps
    pub max_up: f64,
    /// Median RTT
    pub median_rtt: f32,
}

/// Aggregate TCP retransmit summary
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub struct RetransmitSummary {
    /// Total retransmits up
    pub up: i32,
    /// Total retransmits down
    pub down: i32,
    /// TCP packet count up
    pub tcp_up: u64,
    /// TCP packet count down
    pub tcp_down: u64,
}

/// Search result entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub enum SearchResultEntry {
    /// Circuit result
    Circuit {
        /// Circuit ID
        id: String,
        /// Circuit name
        name: String,
    },
    /// Device result
    Device {
        /// Circuit ID
        circuit_id: String,
        /// Device name
        name: String,
        /// Circuit name
        circuit_name: String,
    },
    /// Site result
    Site {
        /// Site index
        idx: usize,
        /// Site name
        name: String,
    },
}

/// Warning level for global warnings
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub enum WarningLevel {
    /// Informational warning
    Info,
    /// Warning-level issue
    Warning,
    /// Error-level issue
    Error,
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

    /// Provides site-level heatmaps.
    SiteHeatmaps(Vec<SiteHeatmapData>),

    /// Provides ASN-level heatmaps.
    AsnHeatmaps(Vec<AsnHeatmapData>),

    /// Provides the global (roll-up) heatmap.
    GlobalHeatmap(HeatmapBlocks),

    /// Provides headline metrics for the Executive Summary page.
    ExecutiveSummaryHeader(ExecutiveSummaryHeader),

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

    /// Stormguard debug snapshot
    StormguardDebug(Vec<StormguardDebugEntry>),

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

    /// List of global warnings
    GlobalWarnings(Vec<(WarningLevel, String)>),

    /// Device counts (shaped + unknown)
    DeviceCounts(DeviceCounts),

    /// Circuit counts (active + configured)
    CircuitCount(CircuitCount),

    /// Flow map points (lat/lon endpoints)
    FlowMap(Vec<FlowMapPoint>),

    /// ASN list (recent flows)
    AsnList(Vec<AsnListEntry>),

    /// Country list (recent flows)
    CountryList(Vec<CountryListEntry>),

    /// Protocol list (recent flows)
    ProtocolList(Vec<ProtocolListEntry>),

    /// ASN flow timeline
    AsnFlowTimeline(Vec<FlowTimelineEntry>),

    /// Country flow timeline
    CountryFlowTimeline(Vec<FlowTimelineEntry>),

    /// Protocol flow timeline
    ProtocolFlowTimeline(Vec<FlowTimelineEntry>),

    /// Scheduler details
    SchedulerDetails(SchedulerDetails),

    /// Queue stats totals (marks/drops)
    QueueStatsTotal(QueueStatsTotal),

    /// Circuit capacity utilization
    CircuitCapacity(Vec<CircuitCapacityRow>),

    /// Tree capacity utilization
    TreeCapacity(Vec<NodeCapacity>),

    /// Retransmit summary
    RetransmitSummary(RetransmitSummary),

    /// Two-level tree summary
    TreeSummaryL2(Vec<(usize, Vec<(usize, lqos_config::NetworkJsonTransport)>)>),

    /// Search results
    SearchResults(Vec<SearchResultEntry>),

    /// Is Insight Enabled?
    InsightStatus(bool),
}
