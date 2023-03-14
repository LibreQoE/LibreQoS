use crate::TcHandle;
use serde::{Deserialize, Serialize};

/// Transmission representation of IP statistics associated
/// with a host.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct IpStats {
  /// The host's IP address, as detected by the XDP program.
  pub ip_address: String,

  /// The host's mapped circuit ID
  pub circuit_id: String,

  /// The current bits-per-second passing through this host. Tuple
  /// 0 is download, tuple 1 is upload.
  pub bits_per_second: (u64, u64),

  /// The current packets-per-second passing through this host. Tuple
  /// 0 is download, tuple 1 is upload.
  pub packets_per_second: (u64, u64),

  /// Median TCP round-trip-time for this host at the current time.
  pub median_tcp_rtt: f32,

  /// Associated TC traffic control handle.
  pub tc_handle: TcHandle,
}

/// Represents an IP Mapping in the XDP IP to TC/CPU mapping system.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
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
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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

/// Defines an IP protocol for display in the flow
/// tracking (Heimdall) system.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum FlowProto {
  /// A TCP flow
  TCP, 
  /// A UDP flow
  UDP, 
  /// An ICMP flow
  ICMP
}

/// Defines the display data for a flow in Heimdall.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FlowTransport {
  /// The Source IP address
  pub src: String,
  /// The Destination IP address
  pub dst: String,
  /// The flow protocol (see `FlowProto`)
  pub proto: FlowProto,
  /// The source port, which is overridden to ICMP code on ICMP flows.
  pub src_port: u16,
  /// The destination port, which isn't useful at all on ICMP flows.
  pub dst_port: u16,
  /// The number of bytes since we started tracking this flow.
  pub bytes: u64,
  /// The number of packets since we started tracking this flow.
  pub packets: u64,
  /// Detected DSCP code if any
  pub dscp: u8,
  /// Detected ECN bit status (0-3)
  pub ecn: u8,
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
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct PacketHeader {
  /// Timestamp (ns since boot)
  pub timestamp: u64,
  /// Source IP
  pub src: String,
  /// Destination IP
  pub dst: String,
  /// Source Port
  pub src_port : u16,
  /// Destination Port
  pub dst_port: u16,
  /// Ip Protocol (see Linux kernel docs)
  pub ip_protocol: u8,
  /// Tos to decode
  pub tos: u8,
  /// Packet Size
  pub size: u32,
}