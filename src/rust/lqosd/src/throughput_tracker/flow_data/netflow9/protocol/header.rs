use lqos_utils::unix_time::time_since_boot;
use nix::sys::time::TimeValLike;

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
        let uptime = time_since_boot().unwrap();

        Self {
            version: (9u16).to_be(),
            count: record_count_including_templates.to_be(),
            sys_uptime: (uptime.num_milliseconds() as u32).to_be(),
            unix_secs: (uptime.num_seconds() as u32).to_be(),
            package_sequence: flow_sequence.to_be(),
            source_id: 0,
        }
    }
}