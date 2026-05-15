//! Definitions for the actual netflow 5 protocol

use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::unix_time::{time_since_boot, unix_now};
use nix::sys::time::TimeValLike;
use std::net::IpAddr;

use crate::throughput_tracker::flow_data::FlowbeeLocalData;

const NETFLOW5_MAX_RECORDS_PER_PACKET: usize = 30;
pub(crate) const NETFLOW5_MAX_FLOWS_PER_PACKET: usize = NETFLOW5_MAX_RECORDS_PER_PACKET / 2;
const NANOS_PER_MILLI: u64 = 1_000_000;

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
        let uptime_ms: u32 = time_since_boot()
            .map(|u| clamp_i64_to_u32("sys_uptime", u.num_milliseconds()))
            .unwrap_or(0);
        let unix_secs = unix_now().unwrap_or(0);

        Self {
            version: (5u16).to_be(),
            count: num_records.to_be(),
            sys_uptime: uptime_ms.to_be(),
            unix_secs: clamp_u64_to_u32("unix_secs", unix_secs).to_be(),
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

fn clamp_i64_to_u32(field: &str, value: i64) -> u32 {
    if value < 0 {
        tracing::warn!("NetFlow5 {field} value {value} is negative; clamping to 0");
        return 0;
    }

    clamp_u64_to_u32(field, value as u64)
}

fn clamp_u64_to_u32(field: &str, value: u64) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => {
            tracing::warn!("NetFlow5 {field} value {value} exceeds u32::MAX; clamping to u32::MAX");
            u32::MAX
        }
    }
}

fn boot_nanos_to_netflow_millis(field: &str, value: u64) -> u32 {
    clamp_u64_to_u32(field, value / NANOS_PER_MILLI)
}

/// Convert a Flowbee key and data to a pair of Netflow 5 records
pub(crate) fn to_netflow_5(
    key: &FlowbeeKey,
    data: &FlowbeeLocalData,
) -> anyhow::Result<(Netflow5Record, Netflow5Record)> {
    let local = key.local_ip.as_ip();
    let remote = key.remote_ip.as_ip();
    if let (IpAddr::V4(local), IpAddr::V4(remote)) = (local, remote) {
        let src_ip = u32::from_ne_bytes(local.octets());
        let dst_ip = u32::from_ne_bytes(remote.octets());
        let d_pkts2 = clamp_u64_to_u32("down packets", data.packets_sent.down).to_be();
        let d_octets2 = clamp_u64_to_u32("down octets", data.bytes_sent.down).to_be();
        let d_pkts = clamp_u64_to_u32("up packets", data.packets_sent.up).to_be();
        let d_octets = clamp_u64_to_u32("up octets", data.bytes_sent.up).to_be();
        let first = boot_nanos_to_netflow_millis("first", data.start_time).to_be();
        let last = boot_nanos_to_netflow_millis("last", data.last_seen).to_be();

        let record = Netflow5Record {
            src_addr: src_ip,
            dst_addr: dst_ip,
            next_hop: 0,
            input: (0u16).to_be(),
            output: (1u16).to_be(),
            d_pkts,
            d_octets,
            first,
            last,
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
            first,
            last,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::throughput_tracker::flow_data::FlowbeeLocalData;
    use lqos_utils::{XdpIpAddress, units::DownUpOrder};
    use std::net::IpAddr;

    fn test_key() -> FlowbeeKey {
        let mut key = FlowbeeKey::default();
        key.local_ip = XdpIpAddress::from_ip(IpAddr::from([192, 0, 2, 10]));
        key.remote_ip = XdpIpAddress::from_ip(IpAddr::from([198, 51, 100, 20]));
        key.src_port = 12345;
        key.dst_port = 443;
        key.ip_protocol = 6;
        key
    }

    fn test_flow_data() -> FlowbeeLocalData {
        FlowbeeLocalData {
            start_time: 1_500_000,
            last_seen: 2_500_000,
            bytes_sent: DownUpOrder::new(u64::from(u32::MAX) + 10, 20),
            packets_sent: DownUpOrder::new(30, u64::from(u32::MAX) + 10),
            rate_estimate_bps: DownUpOrder::new(0, 0),
            display_rate_bps: None,
            tcp_retransmits: DownUpOrder::new(0, 0),
            end_status: 0,
            tos: 0,
            tc_handle: 0,
            cpu: 0,
            circuit_hash: None,
            device_hash: None,
            tcp_info: None,
        }
    }

    #[test]
    fn netflow5_records_clamp_counters_and_use_milliseconds() {
        let key = test_key();
        let data = test_flow_data();

        let (forward, reverse) =
            to_netflow_5(&key, &data).expect("IPv4 flow should convert to NetFlow5 records");

        assert_eq!(u32::from_be(forward.d_pkts), u32::MAX);
        assert_eq!(u32::from_be(forward.d_octets), 20);
        assert_eq!(u32::from_be(reverse.d_pkts), 30);
        assert_eq!(u32::from_be(reverse.d_octets), u32::MAX);
        assert_eq!(u32::from_be(forward.first), 1);
        assert_eq!(u32::from_be(forward.last), 2);
        assert_eq!(u32::from_be(reverse.first), 1);
        assert_eq!(u32::from_be(reverse.last), 2);
    }

    #[test]
    fn netflow5_timestamp_conversion_clamps_after_milliseconds() {
        let too_many_millis = (u64::from(u32::MAX) + 1) * NANOS_PER_MILLI;

        assert_eq!(
            boot_nanos_to_netflow_millis("test timestamp", too_many_millis),
            u32::MAX
        );
    }
}
