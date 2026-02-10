// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

use crate::TcHandle;
use allocative::Allocative;
use lqos_utils::units::DownUpOrder;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Transmission representation of IP statistics associated
/// with a host.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Allocative)]
pub struct IpStats {
    /// The host's IP address, as detected by the XDP program.
    pub ip_address: String,

    /// The host's mapped circuit ID
    pub circuit_id: String,

    /// The current bits-per-second passing through this host.
    pub bits_per_second: DownUpOrder<u64>,

    /// The current packets-per-second passing through this host. Tuple
    /// 0 is download, tuple 1 is upload.
    pub packets_per_second: DownUpOrder<u64>,

    /// Median TCP round-trip-time for this host at the current time.
    pub median_tcp_rtt: f32,

    /// Associated TC traffic control handle.
    pub tc_handle: TcHandle,

    /// TCP Retransmits for this host at the current time.
    pub tcp_retransmits: (f64, f64),
}

/// Represents an IP Mapping in the XDP IP to TC/CPU mapping system.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Allocative)]
pub struct IpMapping {
    /// The mapped IP address. May be IPv4, or IPv6.
    pub ip_address: String,

    /// The CIDR prefix length of the host. Equivalent to the CIDR value
    /// after the /. e.g. `/24`.
    pub prefix_length: u32,

    /// The current TC traffic control handle.
    pub tc_handle: TcHandle,

    /// The CPU index associated with this IP mapping.
    pub cpu: u32,
}

/// Provided for backwards compatibility with `xdp_pping`, with the intent
/// to retire it eventually.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Allocative)]
pub struct XdpPpingResult {
    /// The TC handle in text format. e.g. "1:12"
    pub tc: String,

    /// The average (mean) RTT value for the current sample.
    pub avg: f32,

    /// The minimum RTT value for the current sample.
    pub min: f32,

    /// The maximum RTT value for the current sample.
    pub max: f32,

    /// The median RTT value for the current sample.
    pub median: f32,

    /// The number of samples from which these values were
    /// derived. If 0, the other values are invalid.
    pub samples: u32,
}

/// Extract the 6-bit DSCP and 2-bit ECN code from a TOS field
/// in an IP header.
pub fn tos_parser(tos: u8) -> (u8, u8) {
    // Format: 2 bits of ECN, 6 bits of DSCP
    const ECN: u8 = 0b00000011;
    const DSCP: u8 = 0b11111100;

    let ecn = tos & ECN;
    let dscp = (tos & DSCP) >> 2;
    (dscp, ecn)
}

/// Packet header dump
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Allocative)]
pub struct PacketHeader {
    /// Timestamp (ns since boot)
    pub timestamp: u64,
    /// Source IP
    pub src: String,
    /// Destination IP
    pub dst: String,
    /// Source Port
    pub src_port: u16,
    /// Destination Port
    pub dst_port: u16,
    /// Ip Protocol (see Linux kernel docs)
    pub ip_protocol: u8,
    /// ECN Flag
    pub ecn: u8,
    /// DSCP code
    pub dscp: u8,
    /// Packet Size
    pub size: u32,
    /// TCP Flag Bitset
    pub tcp_flags: u8,
    /// TCP Window Size
    pub tcp_window: u16,
    /// TCP TSVal
    pub tcp_tsval: u32,
    /// TCP ECR val
    pub tcp_tsecr: u32,
}

/// Flowbee protocol enumeration
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Allocative)]
pub enum FlowbeeProtocol {
    /// TCP (type 6)
    TCP,
    /// UDP (type 17)
    UDP,
    /// ICMP (type 1)
    ICMP,
}

impl From<u8> for FlowbeeProtocol {
    fn from(value: u8) -> Self {
        match value {
            6 => Self::TCP,
            17 => Self::UDP,
            _ => Self::ICMP,
        }
    }
}

/// Flowbee: a complete flow data, combining key and data.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Allocative)]
pub struct FlowbeeSummaryData {
    /// Mapped `XdpIpAddress` source for the flow.
    pub remote_ip: String,
    /// Mapped `XdpIpAddress` destination for the flow
    pub local_ip: String,
    /// Source port number, or ICMP type.
    pub src_port: u16,
    /// Destination port number.
    pub dst_port: u16,
    /// IP protocol (see the Linux kernel!)
    pub ip_protocol: FlowbeeProtocol,
    /// Padding to align the structure to 16 bytes.
    /// Time (nanos) when the connection was established
    pub start_time: u64,
    /// Time (nanos) when the connection was last seen
    pub last_seen: u64,
    /// Bytes transmitted
    pub bytes_sent: DownUpOrder<u64>,
    /// Packets transmitted
    pub packets_sent: DownUpOrder<u64>,
    /// Rate estimate
    pub rate_estimate_bps: DownUpOrder<u32>,
    /// TCP Retransmission count (also counts duplicates)
    pub tcp_retransmits: DownUpOrder<u16>,
    /// Has the connection ended?
    /// 0 = Alive, 1 = FIN, 2 = RST
    pub end_status: u8,
    /// Raw IP TOS
    pub tos: u8,
    /// Raw TCP flags
    pub flags: u8,
    /// Recent RTT median
    pub rtt_nanos: DownUpOrder<u64>,
    /// Remote ASN
    pub remote_asn: u32,
    /// Remote ASN Name
    pub remote_asn_name: String,
    /// Remote ASN Country
    pub remote_asn_country: String,
    /// Analysis
    pub analysis: String,
    /// Circuit ID
    pub circuit_id: String,
    /// Circuit Name
    pub circuit_name: String,
}

/// Circuit statistics for transmit
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Allocative)]
pub struct Circuit {
    /// The IP address of the host.
    pub ip: IpAddr,
    /// Current bytes-per-second passing through this host.
    pub bytes_per_second: DownUpOrder<u64>,
    /// Median latency for this host at the current time.
    pub median_latency: Option<f32>,
    /// Current RTT p50 (nanoseconds), per direction.
    #[serde(default)]
    pub rtt_current_p50_nanos: DownUpOrder<Option<u64>>,
    /// Current RTT p95 (nanoseconds), per direction.
    #[serde(default)]
    pub rtt_current_p95_nanos: DownUpOrder<Option<u64>>,
    /// Total (lifetime) RTT p50 (nanoseconds), per direction.
    #[serde(default)]
    pub rtt_total_p50_nanos: DownUpOrder<Option<u64>>,
    /// Total (lifetime) RTT p95 (nanoseconds), per direction.
    #[serde(default)]
    pub rtt_total_p95_nanos: DownUpOrder<Option<u64>>,
    /// QoO score (0..100), per direction.
    #[serde(default)]
    pub qoo: DownUpOrder<Option<f32>>,
    /// TCP Retransmits for this host at the current time.
    pub tcp_retransmits: DownUpOrder<u64>,
    /// The number of TCP packets per second.
    pub tcp_packets: DownUpOrder<u64>,
    /// The mapped circuit ID
    pub circuit_id: Option<String>,
    /// The mapped device ID
    pub device_id: Option<String>,
    /// The parent node of the device
    pub parent_node: Option<String>,
    /// The circuit name
    pub circuit_name: Option<String>,
    /// The device name
    pub device_name: Option<String>,
    /// The current plan for this circuit.
    pub plan: DownUpOrder<f32>,
    /// The last time this host was seen, in nanoseconds since boot.
    pub last_seen_nanos: u64,
}
