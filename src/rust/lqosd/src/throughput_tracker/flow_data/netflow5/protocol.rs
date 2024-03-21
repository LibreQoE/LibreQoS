//! Definitions for the actual netflow 5 protocol

use std::net::IpAddr;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::unix_time::time_since_boot;
use nix::sys::time::TimeValLike;

use crate::throughput_tracker::flow_data::FlowbeeLocalData;

/// Standard Netflow 5 header
#[repr(C)]
pub(crate) struct Netflow5Header {
    pub(crate) version: u16,
    pub(crate) count: u16,
    pub(crate) sys_uptime: u32,
    pub(crate) unix_secs: u32,
    pub(crate) unix_nsecs: u32,
    pub(crate) flow_sequence: u32,
    pub(crate) engine_type: u8,
    pub(crate) engine_id: u8,
    pub(crate) sampling_interval: u16,
}

impl Netflow5Header {
    /// Create a new Netflow 5 header
    pub(crate) fn new(flow_sequence: u32, num_records: u16) -> Self {
        let uptime = time_since_boot().unwrap();

        Self {
            version: (5u16).to_be(),
            count: num_records.to_be(),
            sys_uptime: (uptime.num_milliseconds() as u32).to_be(),
            unix_secs: (uptime.num_seconds() as u32).to_be(),
            unix_nsecs: 0,
            flow_sequence,
            engine_type: 0,
            engine_id: 0,
            sampling_interval: 0,
        }
    }
}

/// Standard Netflow 5 record
#[repr(C)]
pub(crate) struct Netflow5Record {
    pub(crate) src_addr: u32,
    pub(crate) dst_addr: u32,
    pub(crate) next_hop: u32,
    pub(crate) input: u16,
    pub(crate) output: u16,
    pub(crate) d_pkts: u32,
    pub(crate) d_octets: u32,
    pub(crate) first: u32,
    pub(crate) last: u32,
    pub(crate) src_port: u16,
    pub(crate) dst_port: u16,
    pub(crate) pad1: u8,
    pub(crate) tcp_flags: u8,
    pub(crate) prot: u8,
    pub(crate) tos: u8,
    pub(crate) src_as: u16,
    pub(crate) dst_as: u16,
    pub(crate) src_mask: u8,
    pub(crate) dst_mask: u8,
    pub(crate) pad2: u16,
}

/// Convert a Flowbee key and data to a pair of Netflow 5 records
pub(crate) fn to_netflow_5(key: &FlowbeeKey, data: &FlowbeeLocalData) -> anyhow::Result<(Netflow5Record, Netflow5Record)> {
    // TODO: Detect overflow
    let local = key.local_ip.as_ip();
    let remote = key.remote_ip.as_ip();
    if let (IpAddr::V4(local), IpAddr::V4(remote)) = (local, remote) {
        let src_ip = u32::from_ne_bytes(local.octets());
        let dst_ip = u32::from_ne_bytes(remote.octets());
        // Convert d_pkts to network order
        let d_pkts = (data.packets_sent[0] as u32).to_be();
        let d_octets = (data.bytes_sent[0] as u32).to_be();
        let d_pkts2 = (data.packets_sent[1] as u32).to_be();
        let d_octets2 = (data.bytes_sent[1] as u32).to_be();

        let record = Netflow5Record {
            src_addr: src_ip,
            dst_addr: dst_ip,
            next_hop: 0,
            input: (0u16).to_be(),
            output: (1u16).to_be(),
            d_pkts,
            d_octets,
            first: ((data.start_time  / 1_000_000) as u32).to_be(), // Convert to milliseconds
            last: ((data.last_seen / 1_000_000) as u32).to_be(), // Convert to milliseconds
            src_port: key.src_port.to_be(),
            dst_port: key.dst_port.to_be(),
            pad1: 0,
            tcp_flags: 0,
            prot: key.ip_protocol.to_be(),
            tos: 0,
            src_as: 0,
            dst_as: 0,
            src_mask: 0,
            dst_mask: 0,
            pad2: 0,
        };

        let record2 = Netflow5Record {
            src_addr: dst_ip,
            dst_addr: src_ip,
            next_hop: 0,
            input: 1,
            output: 0,
            d_pkts: d_pkts2,
            d_octets: d_octets2,
            first: data.start_time as u32, // Convert to milliseconds
            last: data.last_seen as u32, // Convert to milliseconds
            src_port: key.dst_port.to_be(),
            dst_port: key.src_port.to_be(),
            pad1: 0,
            tcp_flags: 0,
            prot: key.ip_protocol.to_be(),
            tos: 0,
            src_as: 0,
            dst_as: 0,
            src_mask: 0,
            dst_mask: 0,
            pad2: 0,
        };

        Ok((record, record2))
    } else {
        Err(anyhow::anyhow!("Only IPv4 is supported"))
    }
}