//! The Heimdall wire event types (K3-A: relocated out of the kernel perf
//! interface so the bridge session never imports a kernel-transport module).
//!
//! `HeimdallEvent` is the eBPF `heimdall_event` layout; `PACKET_OCTET_SIZE`
//! is the inner capture prefix. Both are consumed by the bridge capture
//! path (`pcap.rs`, `timeline.rs`) as well as the kernel perf callback.

use lqos_utils::XdpIpAddress;
use zerocopy::FromBytes;

/// This constant MUST exactly match PACKET_OCTET_STATE in heimdall.h
pub(crate) const PACKET_OCTET_SIZE: usize = 128;

/// A representation of the eBPF `heimdall_event` type.
/// This is the type that is sent from the eBPF program to userspace.
/// It is a representation of the `heimdall_event` type in heimdall.h
#[derive(FromBytes, Debug, Clone, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct HeimdallEvent {
    /// Timestamp of the event, in nanoseconds since boot time.
    pub timestamp: u64,
    /// Source IP address
    pub src: XdpIpAddress,
    /// Destination IP address
    pub dst: XdpIpAddress,
    /// Source port number, or ICMP type.
    pub src_port: u16,
    /// Destination port number.
    pub dst_port: u16,
    /// IP protocol number
    pub ip_protocol: u8,
    /// IP header TOS value
    pub tos: u8,
    /// Total size of the packet, in bytes
    pub size: u32,
    /// TCP flags
    pub tcp_flags: u8,
    /// TCP window size
    pub tcp_window: u16,
    /// TCP sequence number
    pub tcp_tsval: u32,
    /// TCP acknowledgement number
    pub tcp_tsecr: u32,
    /// Raw packet data
    pub packet_data: [u8; PACKET_OCTET_SIZE],
}
