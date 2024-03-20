use super::QueueStoreTransit;
use crate::{
  ip_stats::{FlowbeeSummaryData, PacketHeader}, IpMapping, IpStats, XdpPpingResult,
};
use lts_client::transport_data::{StatsTotals, StatsHost, StatsTreeNode};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// A `BusResponse` object represents a single
/// reply generated from a `BusRequest`, and batched
/// inside a `BusReply`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
    bits_per_second: (u64, u64),

    /// In pps
    packets_per_second: (u64, u64),

    /// How much of the response has been subject to the shaper?
    shaped_bits_per_second: (u64, u64),
  },

  /// Provides a list of ALL mapped hosts traffic counters,
  /// listing the IP Address and upload/download in a tuple.
  HostCounters(Vec<(IpAddr, u64, u64)>),

  /// Provides the Top N downloaders IP stats.
  TopDownloaders(Vec<IpStats>),

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

  /// Statistics from lqosd
  LqosdStats {
    /// Number of bus requests handled
    bus_requests: u64,
    /// Us to poll hosts
    time_to_poll_hosts: u64,
    /// High traffic watermark
    high_watermark: (u64, u64),
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

  /// Long-term stats top-level totals
  LongTermTotals(StatsTotals),

  /// Long-term stats host totals
  LongTermHosts(Vec<StatsHost>),

  /// Long-term stats tree
  LongTermTree(Vec<StatsTreeNode>),

  /// All Active Flows (Not Recommended - Debug Use)
  AllActiveFlows(Vec<FlowbeeSummaryData>),

  /// Count active flows
  CountActiveFlows(u64),

  /// Top Flopws
  TopFlows(Vec<FlowbeeSummaryData>),

  /// Flows by IP
  FlowsByIp(Vec<FlowbeeSummaryData>),

  /// Current endpoints by country
  CurrentEndpointsByCountry(Vec<(String, [u64; 2], [f32; 2])>),

  /// Current Lat/Lon of endpoints
  CurrentLatLon(Vec<(f64, f64, String, u64, f32)>),

  /// Summary of Ether Protocol
  EtherProtocols{
    /// Number of IPv4 Bytes
    v4_bytes: [u64; 2],
    /// Number of IPv6 Bytes
    v6_bytes: [u64; 2],
    /// Number of IPv4 Packets
    v4_packets: [u64; 2],
    /// Number of IPv6 Packets
    v6_packets: [u64; 2],
    /// Number of IPv4 Flows
    v4_rtt: [u64; 2],
    /// Number of IPv6 Flows
    v6_rtt: [u64; 2],
  },
  
  /// Summary of IP Protocols
  IpProtocols(Vec<(String, (u64, u64))>),
}
