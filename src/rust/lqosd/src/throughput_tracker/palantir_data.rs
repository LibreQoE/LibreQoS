use std::time::Duration;

use dashmap::DashMap;
use lqos_sys::{PalantirData, PalantirKey};
use lqos_utils::unix_time::time_since_boot;
use once_cell::sync::Lazy;

use crate::stats::FLOWS_TRACKED;

pub(crate) static PALANTIR: Lazy<PalantirMonitor> =
  Lazy::new(PalantirMonitor::new);

pub(crate) struct PalantirMonitor {
  pub(crate) data: DashMap<PalantirKey, FlowData>,
}

#[derive(Default)]
pub(crate) struct FlowData {
  last_seen: u64,
  bytes: u64,
  packets: u64,
}

impl PalantirMonitor {
  fn new() -> Self {
    Self { data: DashMap::new() }
  }

  fn combine_flows(values: &[PalantirData]) -> FlowData {
    let mut result = FlowData::default();
    let mut ls = 0;
    values.iter().for_each(|v| {
      result.bytes += v.bytes;
      result.packets += v.packets;
      if v.last_seen > ls {
        ls = v.last_seen;
      }
    });
    result.last_seen = ls;
    result
  }

  pub(crate) fn ingest(&self, key: &PalantirKey, values: &[PalantirData]) {
    if let Some(five_minutes_ago_nanoseconds) = Self::get_expire_time() {
      let combined = Self::combine_flows(values);
      if combined.last_seen > five_minutes_ago_nanoseconds {
        if let Some(mut flow) = self.data.get_mut(key) {
          // Update
          flow.bytes += combined.bytes;
          flow.packets += combined.packets;
          flow.last_seen = combined.last_seen;
        } else {
          // Insert
          self.data.insert(key.clone(), combined);
        }
      }
    }
  }

  fn get_expire_time() -> Option<u64> {
    let boot_time = time_since_boot();
    if let Ok(boot_time) = boot_time {
      let time_since_boot = Duration::from(boot_time);
      let five_minutes_ago =
        time_since_boot.saturating_sub(Duration::from_secs(300));
      let five_minutes_ago_nanoseconds = five_minutes_ago.as_nanos() as u64;
      Some(five_minutes_ago_nanoseconds)
    } else {
      None
    }
  }

  pub(crate) fn expire(&self) {
    if let Some(five_minutes_ago_nanoseconds) = Self::get_expire_time() {
      self.data.retain(|_k, v| v.last_seen > five_minutes_ago_nanoseconds);
    }
    FLOWS_TRACKED.store(self.data.len() as u64, std::sync::atomic::Ordering::Relaxed);
  }
}
