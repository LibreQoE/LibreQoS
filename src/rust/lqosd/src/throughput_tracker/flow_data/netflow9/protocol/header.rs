use lqos_utils::unix_time::{time_since_boot, unix_now};
use nix::sys::time::TimeValLike;

use crate::throughput_tracker::flow_data::netflow_common::{clamp_i64_to_u32, clamp_u64_to_u32};

#[repr(C)]
pub(crate) struct Netflow9Header {
    pub(crate) version: u16,
    pub(crate) count: u16,
    pub(crate) sys_uptime: u32,
    pub(crate) unix_secs: u32,
    pub(crate) package_sequence: u32,
    pub(crate) source_id: u32,
}

impl Netflow9Header {
    /// Create a new Netflow 9 header
    pub(crate) fn new(flow_sequence: u32, record_count_including_templates: u16) -> Self {
        let uptime_ms = time_since_boot().map(|u| u.num_milliseconds()).unwrap_or(0);
        let unix_secs = unix_now().unwrap_or(0);

        Self::from_times(
            flow_sequence,
            record_count_including_templates,
            uptime_ms,
            unix_secs,
        )
    }

    fn from_times(
        flow_sequence: u32,
        record_count_including_templates: u16,
        uptime_ms: i64,
        unix_secs: u64,
    ) -> Self {
        Self {
            version: (9u16).to_be(),
            count: record_count_including_templates.to_be(),
            sys_uptime: clamp_i64_to_u32("NetFlow9", "sys_uptime", uptime_ms).to_be(),
            unix_secs: clamp_u64_to_u32("NetFlow9", "unix_secs", unix_secs).to_be(),
            package_sequence: flow_sequence.to_be(),
            source_id: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netflow9_header_clamps_large_times() {
        let header =
            Netflow9Header::from_times(10, 2, i64::from(u32::MAX) + 1, u64::from(u32::MAX) + 1);

        assert_eq!(u32::from_be(header.sys_uptime), u32::MAX);
        assert_eq!(u32::from_be(header.unix_secs), u32::MAX);
        assert_eq!(u32::from_be(header.package_sequence), 10);
    }

    #[test]
    fn netflow9_header_clamps_negative_uptime_to_zero() {
        let header = Netflow9Header::from_times(10, 2, -1, 1);

        assert_eq!(u32::from_be(header.sys_uptime), 0);
        assert_eq!(u32::from_be(header.unix_secs), 1);
    }
}
