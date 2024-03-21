//! Data structures for the Flowbee eBPF program.

use lqos_utils::XdpIpAddress;
use zerocopy::FromBytes;

/// Representation of the eBPF `flow_key_t` type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, FromBytes)]
#[repr(C)]
pub struct FlowbeeKey {
    /// Mapped `XdpIpAddress` source for the flow.
    pub remote_ip: XdpIpAddress,
    /// Mapped `XdpIpAddress` destination for the flow
    pub local_ip: XdpIpAddress,
    /// Source port number, or ICMP type.
    pub src_port: u16,
    /// Destination port number.
    pub dst_port: u16,
    /// IP protocol (see the Linux kernel!)
    pub ip_protocol: u8,
    /// Padding to align the structure to 16 bytes.
    padding: u8,
    padding1: u8,
    padding2: u8,
}

/// Mapped representation of the eBPF `flow_data_t` type.
#[derive(Debug, Clone, Default, FromBytes)]
#[repr(C)]
pub struct FlowbeeData {
    /// Time (nanos) when the connection was established
    pub start_time: u64,
    /// Time (nanos) when the connection was last seen
    pub last_seen: u64,
    /// Bytes transmitted
    pub bytes_sent: [u64; 2],
    /// Packets transmitted
    pub packets_sent: [u64; 2],
    /// Clock for the next rate estimate
    pub next_count_time: [u64; 2],
    /// Clock for the previous rate estimate
    pub last_count_time: [u64; 2],
    /// Bytes at the next rate estimate
    pub next_count_bytes: [u64; 2],
    /// Rate estimate
    pub rate_estimate_bps: [u32; 2],
    /// Sequence number of the last packet
    pub last_sequence: [u32; 2],
    /// Acknowledgement number of the last packet
    pub last_ack: [u32; 2],
    /// TCP Retransmission count (also counts duplicates)
    pub tcp_retransmits: [u16; 2],
    /// Timestamp values
    pub tsval: [u32; 2],
    /// Timestamp echo values
    pub tsecr: [u32; 2],
    /// When did the timestamp change?
    pub ts_change_time: [u64; 2],
    /// Has the connection ended?
    /// 0 = Alive, 1 = FIN, 2 = RST
    pub end_status: u8,
    /// Raw IP TOS
    pub tos: u8,
    /// Raw TCP flags
    pub flags: u8,
    /// Padding.
    pub padding: u8,
}
