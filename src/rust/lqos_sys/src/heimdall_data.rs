use lqos_utils::XdpIpAddress;
use zerocopy::FromBytes;

/// Representation of the eBPF `heimdall_key` type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, FromBytes)]
#[repr(C)]
pub struct HeimdallKey {
  /// Mapped `XdpIpAddress` source for the flow.
  pub src_ip: XdpIpAddress,
  /// Mapped `XdpIpAddress` destination for the flow
  pub dst_ip: XdpIpAddress,
  /// IP protocol (see the Linux kernel!)
  pub ip_protocol: u8,
  /// Source port number, or ICMP type.
  pub src_port: u16,
  /// Destination port number.
  pub dst_port: u16,
  _padding: u8,
}

/// Mapped representation of the eBPF `heimdall_data` type.
#[derive(Debug, Clone, Default, FromBytes)]
#[repr(C)]
pub struct HeimdallData {
  /// Last seen, in nanoseconds (since boot time).
  pub last_seen: u64,
  /// Number of bytes since the flow started being tracked
  pub bytes: u64,
  /// Number of packets since the flow started being tracked
  pub packets: u64,
  /// IP header TOS value
  pub tos: u8,
}