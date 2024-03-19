use lqos_bus::TcHandle;
use super::flow_data::RttData;

#[derive(Debug)]
pub(crate) struct ThroughputEntry {
  pub(crate) circuit_id: Option<String>,
  pub(crate) network_json_parents: Option<Vec<usize>>,
  pub(crate) first_cycle: u64,
  pub(crate) most_recent_cycle: u64,
  pub(crate) bytes: (u64, u64),
  pub(crate) packets: (u64, u64),
  pub(crate) prev_bytes: (u64, u64),
  pub(crate) prev_packets: (u64, u64),
  pub(crate) bytes_per_second: (u64, u64),
  pub(crate) packets_per_second: (u64, u64),
  pub(crate) tc_handle: TcHandle,
  pub(crate) recent_rtt_data: [RttData; 60],
  pub(crate) last_fresh_rtt_data_cycle: u64,
  pub(crate) last_seen: u64, // Last seen in kernel time since boot
  pub(crate) tcp_retransmits: (u64, u64),
  pub(crate) last_tcp_retransmits: (u64, u64),
}

impl ThroughputEntry {
  /// Calculate the median latency from the recent_rtt_data
  /// Returns an optional, because there might not be any
  /// data to track.
  /// Also explicitly rejects 0 values, and flows that have
  /// less than 1 Mb of data---they are usually long-polling.
  pub(crate) fn median_latency(&self) -> Option<f32> {
    // Reject sub 1Mb flows
    if self.bytes.0 < 1_000_000 || self.bytes.1 < 1_000_000 {
      return None;
    }

    let mut shifted: Vec<f32> = self
      .recent_rtt_data
      .iter()
      .filter(|n| n.as_nanos() != 0)
      .map(|n| n.as_millis() as f32)
      .collect();
    if shifted.len() < 5 {
      return None;
    }
    shifted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Some(shifted[shifted.len() / 2])
  }
}
