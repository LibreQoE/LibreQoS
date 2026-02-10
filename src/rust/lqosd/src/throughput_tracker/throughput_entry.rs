use super::flow_data::{RttBuffer, RttData};
use allocative_derive::Allocative;
use lqos_bus::TcHandle;
use lqos_utils::qoo::QoqScores;
use lqos_utils::units::DownUpOrder;

#[derive(Debug, Allocative)]
pub(crate) struct ThroughputEntry {
    pub(crate) circuit_id: Option<String>,
    pub(crate) circuit_hash: Option<i64>,
    pub(crate) network_json_parents: Option<Vec<usize>>,
    pub(crate) first_cycle: u64,
    pub(crate) most_recent_cycle: u64,
    pub(crate) bytes: DownUpOrder<u64>,        // 0 DL, 1 UL
    pub(crate) packets: DownUpOrder<u64>,      // 0 DL, 1 UL
    pub(crate) tcp_packets: DownUpOrder<u64>,  // 0 DL, 1 UL
    pub(crate) udp_packets: DownUpOrder<u64>,  // 0 DL, 1 UL
    pub(crate) icmp_packets: DownUpOrder<u64>, // 0 DL, 1 UL
    pub(crate) prev_bytes: DownUpOrder<u64>,   // Has to mirror
    pub(crate) prev_packets: DownUpOrder<u64>,
    pub(crate) prev_tcp_packets: DownUpOrder<u64>,
    pub(crate) prev_udp_packets: DownUpOrder<u64>,
    pub(crate) prev_icmp_packets: DownUpOrder<u64>,
    pub(crate) bytes_per_second: DownUpOrder<u64>,
    pub(crate) packets_per_second: DownUpOrder<u64>,
    pub(crate) tc_handle: TcHandle,
    pub(crate) rtt_buffer: RttBuffer,
    pub(crate) recent_rtt_data: [RttData; 60],
    pub(crate) last_fresh_rtt_data_cycle: u64,
    pub(crate) last_seen: u64, // Last seen in kernel time since boot
    pub(crate) tcp_retransmits: DownUpOrder<u64>,
    pub(crate) prev_tcp_retransmits: DownUpOrder<u64>,
    pub(crate) qoq: QoqScores,
}

impl ThroughputEntry {
    /// Calculate the median latency from the recent_rtt_data
    /// Returns an optional, because there might not be any
    /// data to track.
    /// Also explicitly rejects 0 values, and flows that have
    /// less than 1 Mb of data---they are usually long-polling.
    pub(crate) fn median_latency(&self) -> Option<f32> {
        // Reject sub 1Mb flows
        if self.bytes.both_less_than(1_000_000) {
            return None;
        }

        let mut shifted: Vec<f32> = self
            .recent_rtt_data
            .iter()
            .filter(|n| n.as_nanos() != 0)
            .map(|n| n.as_millis() as f32)
            .collect();
        if shifted.len() < 2 {
            return None;
        }
        shifted.sort_by(|a, b| a.total_cmp(b));
        Some(shifted[shifted.len() / 2])
    }
}
