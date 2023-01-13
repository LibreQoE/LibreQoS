use lqos_bus::TcHandle;

#[derive(Debug)]
pub(crate) struct ThroughputEntry {
    pub(crate) first_cycle: u64,
    pub(crate) most_recent_cycle: u64,
    pub(crate) bytes: (u64, u64),
    pub(crate) packets: (u64, u64),
    pub(crate) prev_bytes: (u64, u64),
    pub(crate) prev_packets: (u64, u64),
    pub(crate) bytes_per_second: (u64, u64),
    pub(crate) packets_per_second: (u64, u64),
    pub(crate) tc_handle: TcHandle,
    pub(crate) recent_rtt_data: [u32; 60],
    pub(crate) last_fresh_rtt_data_cycle: u64,
}

impl ThroughputEntry {
    pub(crate) fn median_latency(&self) -> f32 {
        let mut shifted: Vec<f32> = self
            .recent_rtt_data
            .iter()
            .filter(|n| **n != 0)
            .map(|n| *n as f32 / 100.0)
            .collect();
        if shifted.is_empty() {
            return 0.0;
        }
        shifted.sort_by(|a, b| a.partial_cmp(&b).unwrap());
        shifted[shifted.len() / 2]
    }
}
