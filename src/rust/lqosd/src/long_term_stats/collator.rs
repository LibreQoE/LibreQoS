use lqos_utils::unix_time::unix_now;

use super::{
  collation_utils::{MinMaxAvg, MinMaxAvgPair},
  submission::new_submission, tree::{NetworkTreeEntry, get_network_tree},
};
use crate::long_term_stats::data_collector::SESSION_BUFFER;
use std::{collections::HashMap, net::IpAddr};

#[derive(Debug, Clone)]
pub(crate) struct StatsSubmission {
  pub(crate) timestamp: u64,
  pub(crate) bits_per_second: MinMaxAvgPair<u64>,
  pub(crate) shaped_bits_per_second: MinMaxAvgPair<u64>,
  pub(crate) packets_per_second: MinMaxAvgPair<u64>,
  pub(crate) hosts: Vec<SubmissionHost>,
  pub(crate) tree: Vec<NetworkTreeEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct SubmissionHost {
  pub(crate) circuit_id: String,
  pub(crate) ip_address: IpAddr,
  pub(crate) bits_per_second: MinMaxAvgPair<u64>,
  pub(crate) median_rtt: MinMaxAvg<u32>,
  pub(crate) tree_parent_indices: Vec<usize>,
}

impl From<StatsSubmission> for lqos_bus::long_term_stats::StatsTotals {
  fn from(value: StatsSubmission) -> Self {
    Self {
      bits: value.bits_per_second.into(),
      shaped_bits: value.shaped_bits_per_second.into(),
      packets: value.packets_per_second.into(),
    }
  }
}

impl From<MinMaxAvgPair<u64>> for lqos_bus::long_term_stats::StatsSummary {
  fn from(value: MinMaxAvgPair<u64>) -> Self {
    Self {
      min: (value.down.min, value.up.min),
      max: (value.down.max, value.up.max),
      avg: (value.down.avg, value.up.avg),
    }
  }
}

impl From<MinMaxAvg<u32>> for lqos_bus::long_term_stats::StatsRttSummary {
  fn from(value: MinMaxAvg<u32>) -> Self {
    Self { min: value.min, max: value.max, avg: value.avg }
  }
}

impl From<SubmissionHost> for lqos_bus::long_term_stats::StatsHost {
  fn from(value: SubmissionHost) -> Self {
    Self {
      circuit_id: value.circuit_id.to_string(),
      ip_address: value.ip_address.to_string(),
      bits: value.bits_per_second.into(),
      rtt: value.median_rtt.into(),
      tree_indices: value.tree_parent_indices,
    }
  }
}

/// Every (n) seconds, collate the accumulated stats buffer
/// into a current statistics block (min/max/avg format)
/// ready for submission to the stats system.
///
/// (n) is defined in /etc/lqos.conf in the `collation_period_seconds`
/// field of the `[long_term_stats]` section.
pub(crate) async fn collate_stats() {
  // Obtain exclusive access to the session
  let mut writer = SESSION_BUFFER.lock().await;
  if writer.is_empty() {
    // Nothing to do - so exit
    return;
  }

  // Collate total stats for the period
  let bps: Vec<(u64, u64)> =
    writer.iter().map(|e| e.bits_per_second).collect();
  let pps: Vec<(u64, u64)> =
    writer.iter().map(|e| e.packets_per_second).collect();
  let sbps: Vec<(u64, u64)> =
    writer.iter().map(|e| e.shaped_bits_per_second).collect();
  let bits_per_second = MinMaxAvgPair::from_slice(&bps);
  let packets_per_second = MinMaxAvgPair::from_slice(&pps);
  let shaped_bits_per_second = MinMaxAvgPair::from_slice(&sbps);

  let mut submission = StatsSubmission {
    timestamp: unix_now().unwrap_or(0),
    bits_per_second,
    shaped_bits_per_second,
    packets_per_second,
    hosts: Vec::new(),
    tree: get_network_tree(),
  };

  // Collate host stats
  let mut host_accumulator =
    HashMap::<(&IpAddr, &String), Vec<(u64, u64, f32, Vec<usize>)>>::new();
  writer.iter().for_each(|session| {
    session.hosts.iter().for_each(|host| {
      if let Some(ha) =
        host_accumulator.get_mut(&(&host.ip_address, &host.circuit_id))
      {
        ha.push((
          host.bits_per_second.0,
          host.bits_per_second.1,
          host.median_rtt,
          host.tree_parent_indices.clone(),
        ));
      } else {
        host_accumulator.insert(
          (&host.ip_address, &host.circuit_id),
          vec![(
            host.bits_per_second.0,
            host.bits_per_second.1,
            host.median_rtt,
            host.tree_parent_indices.clone(),
          )],
        );
      }
    });
  });

  for ((ip, circuit), data) in host_accumulator.iter() {
    let bps: Vec<(u64, u64)> =
      data.iter().map(|(d, u, _rtt, _tree)| (*d, *u)).collect();
    let bps = MinMaxAvgPair::<u64>::from_slice(&bps);
    let fps: Vec<u32> =
      data.iter().map(|(_d, _u, rtt, _tree)| (*rtt * 100.0) as u32).collect();
    let fps = MinMaxAvg::<u32>::from_slice(&fps);
    let tree = data
      .iter()
      .cloned()
      .map(|(_d, _u, _rtt, tree)| tree)
      .next()
      .unwrap_or(Vec::new());
    submission.hosts.push(SubmissionHost {
      circuit_id: circuit.to_string(),
      ip_address: **ip,
      bits_per_second: bps,
      median_rtt: fps,
      tree_parent_indices: tree,
    });
  }

  // Remove all gathered stats
  writer.clear();

  // Drop the lock
  std::mem::drop(writer);

  // Submit
  new_submission(submission).await;
}
