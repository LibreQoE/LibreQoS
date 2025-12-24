// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

use super::QueueStoreTransit;
use crate::{
    Circuit, IpMapping, IpStats, XdpPpingResult,
    ip_stats::{FlowbeeSummaryData, PacketHeader},
};
use allocative::Allocative;
use lqos_utils::units::DownUpOrder;
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

    /// Is Insight Enabled?
    InsightStatus(bool),
}
