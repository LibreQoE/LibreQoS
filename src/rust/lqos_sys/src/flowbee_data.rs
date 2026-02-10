//! Data structures for the Flowbee eBPF program.

use allocative_derive::Allocative;
use lqos_utils::XdpIpAddress;
use lqos_utils::units::DownUpOrder;
use zerocopy::FromBytes;

/// Representation of the eBPF `flow_key_t` type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, FromBytes, Allocative)]
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

/// Representation of the eBPF `tsval_record_buffer_t` type.
#[derive(Debug, Clone, Copy, Default, FromBytes)]
#[repr(C)]
pub struct TsvalRecordBuffer {
    /// Times (nanos since boot) when TSvals were observed.
    pub timestamps: [u64; 2],
    /// TSval values that correspond to `timestamps`.
    pub tsvals: [u32; 2],
}

/// Mapped representation of the eBPF `flow_data_t` type.
#[derive(Debug, Clone, Copy, Default, FromBytes)]
#[repr(C)]
pub struct FlowbeeData {
    /// Time (nanos) when the connection was established
    pub start_time: u64,
    /// Time (nanos) when the connection was last seen
    pub last_seen: u64,
    /// Bytes transmitted
    pub bytes_sent: DownUpOrder<u64>,
    /// Packets transmitted
    pub packets_sent: DownUpOrder<u64>,
    /// Clock for the next rate estimate
    pub next_count_time: DownUpOrder<u64>,
    /// Clock for the previous rate estimate
    pub last_count_time: DownUpOrder<u64>,
    /// Bytes at the next rate estimate
    pub next_count_bytes: DownUpOrder<u64>,
    /// Rate estimate
    pub rate_estimate_bps: DownUpOrder<u32>,
    /// Sequence number of the last packet
    pub last_sequence: DownUpOrder<u32>,
    /// TCP Retransmission count (also counts duplicates)
    pub tcp_retransmits: DownUpOrder<u16>,
    /// Padding to avoid 4-byte hole and push TSval/TSecr entries to new cacheline
    pub padding1: u32,
    /// Timestamp values
    pub tsval: DownUpOrder<u32>,
    /// Timestamp echo values
    pub tsecr: DownUpOrder<u32>,
    /// Tracked TSval samples (used to infer RTT via TSval/TSecr matching).
    pub tsval_tstamps: DownUpOrder<TsvalRecordBuffer>,
    /// Last time we pushed an RTT sample
    pub last_rtt: DownUpOrder<u64>,
    /// Has the connection ended?
    /// 0 = Alive, 1 = FIN, 2 = RST
    pub end_status: u8,
    /// Raw IP TOS
    pub tos: u8,
    /// Raw TCP flags
    pub flags: u8,
    /// Padding.
    pub padding2: [u8; 5],
}

const _: [(); 40] = [(); core::mem::size_of::<FlowbeeKey>()];
const _: [(); 24] = [(); core::mem::size_of::<TsvalRecordBuffer>()];
const _: [(); 48] = [(); core::mem::size_of::<DownUpOrder<TsvalRecordBuffer>>()];
const _: [(); 208] = [(); core::mem::size_of::<FlowbeeData>()];
