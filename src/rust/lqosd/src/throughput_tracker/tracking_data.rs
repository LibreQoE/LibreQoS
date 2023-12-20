use std::sync::atomic::AtomicU64;
use crate::{shaped_devices_tracker::{SHAPED_DEVICES, NETWORK_JSON}, stats::{HIGH_WATERMARK_DOWN, HIGH_WATERMARK_UP}};
use super::{throughput_entry::ThroughputEntry, RETIRE_AFTER_SECONDS};
use dashmap::DashMap;
use lqos_bus::TcHandle;
use lqos_sys::{rtt_for_each, throughput_for_each};
use lqos_utils::XdpIpAddress;

pub struct ThroughputTracker {
  pub(crate) cycle: AtomicU64,
  pub(crate) raw_data: DashMap<XdpIpAddress, ThroughputEntry>,
  pub(crate) bytes_per_second: (AtomicU64, AtomicU64),
  pub(crate) packets_per_second: (AtomicU64, AtomicU64),
  pub(crate) shaped_bytes_per_second: (AtomicU64, AtomicU64),
}

impl ThroughputTracker {
  pub(crate) fn new() -> Self {
    // The capacity should match that found in
    // maximums.h (MAX_TRACKED_IPS), so we grab it
    // from there via the C API.
    Self {
      cycle: AtomicU64::new(RETIRE_AFTER_SECONDS),
      raw_data: DashMap::with_capacity(lqos_sys::max_tracked_ips()),
      bytes_per_second: (AtomicU64::new(0), AtomicU64::new(0)),
      packets_per_second: (AtomicU64::new(0), AtomicU64::new(0)),
      shaped_bytes_per_second: (AtomicU64::new(0), AtomicU64::new(0)),
    }
  }

  pub(crate) fn copy_previous_and_reset_rtt(&self) {
    // Copy previous byte/packet numbers and reset RTT data
    let self_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);
    self.raw_data.iter_mut().for_each(|mut v| {
      if v.first_cycle < self_cycle {
        v.bytes_per_second.0 =
          u64::checked_sub(v.bytes.0, v.prev_bytes.0).unwrap_or(0);
        v.bytes_per_second.1 =
          u64::checked_sub(v.bytes.1, v.prev_bytes.1).unwrap_or(0);
        v.packets_per_second.0 =
          u64::checked_sub(v.packets.0, v.prev_packets.0).unwrap_or(0);
        v.packets_per_second.1 =
          u64::checked_sub(v.packets.1, v.prev_packets.1).unwrap_or(0);
      }
      v.prev_bytes = v.bytes;
      v.prev_packets = v.packets;

      // Roll out stale RTT data
      if self_cycle > RETIRE_AFTER_SECONDS
        && v.last_fresh_rtt_data_cycle < self_cycle - RETIRE_AFTER_SECONDS
      {
        v.recent_rtt_data = [0; 60];
      }
    });
  }

  fn lookup_circuit_id(xdp_ip: &XdpIpAddress) -> Option<String> {
    let mut circuit_id = None;
    let lookup = xdp_ip.as_ipv6();
    let cfg = SHAPED_DEVICES.read().unwrap();
    if let Some((_, id)) = cfg.trie.longest_match(lookup) {
      circuit_id = Some(cfg.devices[*id].circuit_id.clone());
    }
    //println!("{lookup:?} Found circuit_id: {circuit_id:?}");
    circuit_id
  }

  pub(crate) fn get_node_name_for_circuit_id(
    circuit_id: Option<String>,
  ) -> Option<String> {
    if let Some(circuit_id) = circuit_id {
      let shaped = SHAPED_DEVICES.read().unwrap();
      let parent_name = shaped
        .devices
        .iter()
        .find(|d| d.circuit_id == circuit_id)
        .map(|device| device.parent_node.clone());
      //println!("{parent_name:?}");
      parent_name
    } else {
      None
    }
  }

  pub(crate) fn lookup_network_parents(
    circuit_id: Option<String>,
  ) -> Option<Vec<usize>> {
    if let Some(parent) = Self::get_node_name_for_circuit_id(circuit_id) {
      let lock = crate::shaped_devices_tracker::NETWORK_JSON.read().unwrap();
      lock.get_parents_for_circuit_id(&parent)
    } else {
      None
    }
  }

  pub(crate) fn refresh_circuit_ids(&self) {
    self.raw_data.iter_mut().for_each(|mut data| {
      data.circuit_id = Self::lookup_circuit_id(data.key());
      data.network_json_parents =
        Self::lookup_network_parents(data.circuit_id.clone());
    });
  }

  pub(crate) fn apply_new_throughput_counters(
    &self,
  ) {
    let raw_data = &self.raw_data;
    let self_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);
    throughput_for_each(&mut |xdp_ip, counts| {
      if let Some(mut entry) = raw_data.get_mut(xdp_ip) {
        entry.bytes = (0, 0);
        entry.packets = (0, 0);
        for c in counts {
          entry.bytes.0 += c.download_bytes;
          entry.bytes.1 += c.upload_bytes;
          entry.packets.0 += c.download_packets;
          entry.packets.1 += c.upload_packets;
          if c.tc_handle != 0 {
            entry.tc_handle = TcHandle::from_u32(c.tc_handle);
          }
          if c.last_seen != 0 {
            entry.last_seen = c.last_seen;
          }
        }
        if entry.packets != entry.prev_packets {
          entry.most_recent_cycle = self_cycle;

          if let Some(parents) = &entry.network_json_parents {
            let net_json = NETWORK_JSON.read().unwrap();
            net_json.add_throughput_cycle(
              parents,
              (
                entry.bytes.0.saturating_sub(entry.prev_bytes.0),
                entry.bytes.1.saturating_sub(entry.prev_bytes.1),
              ),
            );
          }
        }
      } else {
        let circuit_id = Self::lookup_circuit_id(xdp_ip);
        let mut entry = ThroughputEntry {
          circuit_id: circuit_id.clone(),
          network_json_parents: Self::lookup_network_parents(circuit_id),
          first_cycle: self_cycle,
          most_recent_cycle: 0,
          bytes: (0, 0),
          packets: (0, 0),
          prev_bytes: (0, 0),
          prev_packets: (0, 0),
          bytes_per_second: (0, 0),
          packets_per_second: (0, 0),
          tc_handle: TcHandle::zero(),
          recent_rtt_data: [0; 60],
          last_fresh_rtt_data_cycle: 0,
          last_seen: 0,
        };
        for c in counts {
          entry.bytes.0 += c.download_bytes;
          entry.bytes.1 += c.upload_bytes;
          entry.packets.0 += c.download_packets;
          entry.packets.1 += c.upload_packets;
          if c.tc_handle != 0 {
            entry.tc_handle = TcHandle::from_u32(c.tc_handle);
          }
        }
        raw_data.insert(*xdp_ip, entry);
      }
    });
  }

  pub(crate) fn apply_rtt_data(&self) {
    let self_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);
    rtt_for_each(&mut |ip, rtt| {
      if rtt.has_fresh_data != 0 {
        if let Some(mut tracker) = self.raw_data.get_mut(ip) {
          tracker.recent_rtt_data = rtt.rtt;
          tracker.last_fresh_rtt_data_cycle = self_cycle;
          if let Some(parents) = &tracker.network_json_parents {
            let net_json = NETWORK_JSON.write().unwrap();
            if let Some(rtt) = tracker.median_latency() {
              net_json.add_rtt_cycle(parents, rtt);
            }
          }
        }
      }
    });
  }

  #[inline(always)]
  fn set_atomic_tuple_to_zero(tuple: &(AtomicU64, AtomicU64)) {
    tuple.0.store(0, std::sync::atomic::Ordering::Relaxed);
    tuple.1.store(0, std::sync::atomic::Ordering::Relaxed);
  }

  #[inline(always)]
  fn add_atomic_tuple(tuple: &(AtomicU64, AtomicU64), n: (u64, u64)) {
    let n0 = tuple.0.load(std::sync::atomic::Ordering::Relaxed);
    if let Some(n) = n0.checked_add(n.0) {
      tuple.0.store(n, std::sync::atomic::Ordering::Relaxed);
    }

    let n1 = tuple.1.load(std::sync::atomic::Ordering::Relaxed);
    if let Some(n) = n1.checked_add(n.1) {
      tuple.1.store(n, std::sync::atomic::Ordering::Relaxed);
    }
  }

  pub(crate) fn update_totals(&self) {
    let current_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);
    Self::set_atomic_tuple_to_zero(&self.bytes_per_second);
    Self::set_atomic_tuple_to_zero(&self.packets_per_second);
    Self::set_atomic_tuple_to_zero(&self.shaped_bytes_per_second);
    self
      .raw_data
      .iter()
      .filter(|v| 
        v.most_recent_cycle == current_cycle &&
        v.first_cycle + 2 < current_cycle
      )
      .map(|v| {
        (
          v.bytes.0.saturating_sub(v.prev_bytes.0),
          v.bytes.1.saturating_sub(v.prev_bytes.1),
          v.packets.0.saturating_sub(v.prev_packets.0),
          v.packets.1.saturating_sub(v.prev_packets.1),
          v.tc_handle.as_u32() > 0,
        )
      })
      .for_each(|(bytes_down, bytes_up, packets_down, packets_up, shaped)| {
        Self::add_atomic_tuple(&self.bytes_per_second, (bytes_down, bytes_up));
        Self::add_atomic_tuple(&self.packets_per_second, (packets_down, packets_up));
        if shaped {
          Self::add_atomic_tuple(&self.shaped_bytes_per_second, (bytes_down, bytes_up));
        }
      });

      let current = self.bits_per_second();
      if current.0 < 100000000000  && current.1 < 100000000000 {
        let prev_max = (
          HIGH_WATERMARK_DOWN.load(std::sync::atomic::Ordering::Relaxed),
          HIGH_WATERMARK_UP.load(std::sync::atomic::Ordering::Relaxed),
        );
        if current.0 > prev_max.0 {
          HIGH_WATERMARK_DOWN.store(current.0, std::sync::atomic::Ordering::Relaxed);
        }
        if current.1 > prev_max.1 {
          HIGH_WATERMARK_UP.store(current.1, std::sync::atomic::Ordering::Relaxed);
        }
      }
  }

  pub(crate) fn next_cycle(&self) {
    self.cycle.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
  }

  pub(crate) fn bits_per_second(&self) -> (u64, u64) {
    (self.bytes_per_second.0.load(std::sync::atomic::Ordering::Relaxed) * 8, self.bytes_per_second.1.load(std::sync::atomic::Ordering::Relaxed) * 8)
  }

  pub(crate) fn shaped_bits_per_second(&self) -> (u64, u64) {
    (self.shaped_bytes_per_second.0.load(std::sync::atomic::Ordering::Relaxed) * 8, self.shaped_bytes_per_second.1.load(std::sync::atomic::Ordering::Relaxed) * 8)
  }

  pub(crate) fn packets_per_second(&self) -> (u64, u64) {
    (
      self.packets_per_second.0.load(std::sync::atomic::Ordering::Relaxed),
      self.packets_per_second.1.load(std::sync::atomic::Ordering::Relaxed),
    )
  }

  #[allow(dead_code)]
  pub(crate) fn dump(&self) {
    for v in self.raw_data.iter() {
      let ip = v.key().as_ip();
      log::info!("{:<34}{:?}", ip, v.tc_handle);
    }
  }
}
