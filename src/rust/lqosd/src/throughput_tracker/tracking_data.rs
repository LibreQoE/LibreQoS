use std::{sync::atomic::AtomicU64, time::Duration};
use crate::{shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES}, stats::HIGH_WATERMARK, throughput_tracker::flow_data::{expire_rtt_flows, flowbee_rtt_map}};
use super::{flow_data::{get_flowbee_event_count_and_reset, FlowAnalysis, FlowbeeLocalData, RttData, ALL_FLOWS}, throughput_entry::ThroughputEntry, RETIRE_AFTER_SECONDS};
use dashmap::DashMap;
use fxhash::FxHashMap;
use lqos_bus::TcHandle;
use lqos_sys::{flowbee_data::FlowbeeKey, iterate_flows, throughput_for_each};
use lqos_utils::{unix_time::time_since_boot, XdpIpAddress};
use lqos_utils::units::AtomicDownUp;

pub struct ThroughputTracker {
  pub(crate) cycle: AtomicU64,
  pub(crate) raw_data: DashMap<XdpIpAddress, ThroughputEntry>,
  pub(crate) bytes_per_second: AtomicDownUp,
  pub(crate) packets_per_second: AtomicDownUp,
  pub(crate) shaped_bytes_per_second: AtomicDownUp,
}

impl ThroughputTracker {
  pub(crate) fn new() -> Self {
    // The capacity should match that found in
    // maximums.h (MAX_TRACKED_IPS), so we grab it
    // from there via the C API.
    Self {
      cycle: AtomicU64::new(RETIRE_AFTER_SECONDS),
      raw_data: DashMap::with_capacity(lqos_sys::max_tracked_ips()),
      bytes_per_second: AtomicDownUp::zeroed(),
      packets_per_second: AtomicDownUp::zeroed(),
      shaped_bytes_per_second: AtomicDownUp::zeroed(),
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
        v.recent_rtt_data = [RttData::from_nanos(0); 60];
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
          recent_rtt_data: [RttData::from_nanos(0); 60],
          last_fresh_rtt_data_cycle: 0,
          last_seen: 0,
          tcp_retransmits: (0, 0),
          last_tcp_retransmits: (0, 0),
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

  pub(crate) fn apply_flow_data(
    &self, 
    timeout_seconds: u64,
    _netflow_enabled: bool,
    sender: std::sync::mpsc::Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
  ) {
    //log::debug!("Flowbee events this second: {}", get_flowbee_event_count_and_reset());
    let self_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);

    if let Ok(now) = time_since_boot() {
      let rtt_samples = flowbee_rtt_map();
      get_flowbee_event_count_and_reset();
      let since_boot = Duration::from(now);
      let expire = (since_boot - Duration::from_secs(timeout_seconds)).as_nanos() as u64;

      // Tracker for per-circuit RTT data. We're losing some of the smoothness by sampling
      // every flow; the idea is to combine them into a single entry for the circuit. This
      // should limit outliers.
      let mut rtt_circuit_tracker: FxHashMap<XdpIpAddress, [Vec<RttData>; 2]> = FxHashMap::default();

      // Tracker for TCP retries. We're storing these per second.
      let mut tcp_retries: FxHashMap<XdpIpAddress, [u64; 2]> = FxHashMap::default();

      // Track the expired keys
      let mut expired_keys = Vec::new();

      let mut all_flows_lock = ALL_FLOWS.lock().unwrap();
        
      // Track through all the flows
      iterate_flows(&mut |key, data| {

        if data.end_status == 3 {
          // The flow has been handled already and should be ignored.
          // DO NOT process it again.          
        } else if data.last_seen < expire {
          // This flow has expired but not been handled yet. Add it to the list to be cleaned.
          expired_keys.push(key.clone());
        } else {
          // We have a valid flow, so it needs to be tracked
          if let Some(this_flow) = all_flows_lock.get_mut(&key) {
            this_flow.0.last_seen = data.last_seen;
            this_flow.0.bytes_sent = data.bytes_sent;
            this_flow.0.packets_sent = data.packets_sent;
            this_flow.0.rate_estimate_bps = data.rate_estimate_bps;
            this_flow.0.tcp_retransmits = data.tcp_retransmits;
            this_flow.0.end_status = data.end_status;
            this_flow.0.tos = data.tos;
            this_flow.0.flags = data.flags;
            if let Some([up, down]) = rtt_samples.get(&key) {
              if up.as_nanos() != 0 {
                this_flow.0.rtt[0] = *up;              
              }
              if down.as_nanos() != 0 {
                this_flow.0.rtt[1] = *down;
              }
            }
          } else {
            // Insert it into the map
            let flow_analysis = FlowAnalysis::new(&key);
            all_flows_lock.insert(key.clone(), (data.into(), flow_analysis));
          }

          // TCP - we have RTT data? 6 is TCP
          if key.ip_protocol == 6 && data.end_status == 0 && self.raw_data.contains_key(&key.local_ip) {
              if let Some(rtt) = rtt_samples.get(&key) {
                // Add the RTT data to the per-circuit tracker
                if let Some(tracker) = rtt_circuit_tracker.get_mut(&key.local_ip) {
                  if rtt[0].as_nanos() > 0 {
                    tracker[0].push(rtt[0]);
                  }
                  if rtt[1].as_nanos() > 0 {
                    tracker[1].push(rtt[1]);
                  }
                } else if rtt[0].as_nanos() > 0 || rtt[1].as_nanos() > 0 {
                  rtt_circuit_tracker.insert(key.local_ip, [vec![rtt[0]], vec![rtt[1]]]);
                }
              }

              // TCP Retries
              if let Some(retries) = tcp_retries.get_mut(&key.local_ip) {
                retries[0] += data.tcp_retransmits[0] as u64;
                retries[1] += data.tcp_retransmits[1] as u64;
              } else {
                tcp_retries.insert(key.local_ip, [data.tcp_retransmits[0] as u64, data.tcp_retransmits[1] as u64]);
              }

              if data.end_status != 0 {
                // The flow has ended. We need to remove it from the map.
                expired_keys.push(key.clone());
              }
          }
        }
      }); // End flow iterator

      // Merge in the per-flow RTT data into the per-circuit tracker
      for (local_ip, rtt_data) in rtt_circuit_tracker {
        let mut rtts = rtt_data[0].iter().filter(|r| r.as_nanos() > 0).collect::<Vec<_>>();
        rtts.extend(rtt_data[1].iter().filter(|r| r.as_nanos() > 0));
        if !rtts.is_empty() {
          rtts.sort();
          let median = rtts[rtts.len() / 2];
          if let Some(mut tracker) = self.raw_data.get_mut(&local_ip) {
            // Only apply if the flow has achieved 1 Mbps or more
            if tracker.bytes_per_second.0 + tracker.bytes_per_second.1 > 125000 {
              // Shift left
              for i in 1..60 {
                tracker.recent_rtt_data[i] = tracker.recent_rtt_data[i - 1];
              }
              tracker.recent_rtt_data[0] = *median;
              tracker.last_fresh_rtt_data_cycle = self_cycle;
              if let Some(parents) = &tracker.network_json_parents {
                let net_json = NETWORK_JSON.write().unwrap();
                if let Some(rtt) = tracker.median_latency() {
                  net_json.add_rtt_cycle(parents, rtt);
                }
              }
            }
          }
        }
      }

      // Merge in the TCP retries
      // Reset all entries in the tracker to 0
      for mut circuit in self.raw_data.iter_mut() {
        circuit.tcp_retransmits = (0, 0);
      }
      // Apply the new ones
      for (local_ip, retries) in tcp_retries {
        if let Some(mut tracker) = self.raw_data.get_mut(&local_ip) {
          tracker.tcp_retransmits.0 = retries[0].saturating_sub(tracker.last_tcp_retransmits.0);
          tracker.tcp_retransmits.1 = retries[1].saturating_sub(tracker.last_tcp_retransmits.1);
          tracker.last_tcp_retransmits.0 = retries[0];
          tracker.last_tcp_retransmits.1 = retries[1];
        }
      }

      // Key Expiration
      if !expired_keys.is_empty() {
        for key in expired_keys.iter() {
          // Send it off to netperf for analysis if we are supporting doing so.
          if let Some(d) = all_flows_lock.get(&key) {
            let _ = sender.send((key.clone(), (d.0.clone(), d.1.clone())));
          }
          // Remove the flow from circulation
          all_flows_lock.remove(&key);
        }

        let ret = lqos_sys::end_flows(&mut expired_keys);
        if let Err(e) = ret {
          log::warn!("Failed to end flows: {:?}", e);
        }
      }

      // Cleaning run
      all_flows_lock.retain(|_k,v| v.0.last_seen >= expire);
      expire_rtt_flows();
    }
  }

  pub(crate) fn update_totals(&self) {
    let current_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);
    self.bytes_per_second.set_to_zero();
    self.packets_per_second.set_to_zero();
    self.shaped_bytes_per_second.set_to_zero();
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
        self.bytes_per_second.checked_add_tuple((bytes_down, bytes_up));
        self.packets_per_second.checked_add_tuple((packets_down, packets_up));
        if shaped {
          self.shaped_bytes_per_second.checked_add_tuple((bytes_down, bytes_up));
        }
      });

      let current = self.bits_per_second();
      if current.0 < 100000000000  && current.1 < 100000000000 {
        let prev_max = (
          HIGH_WATERMARK.get_down(),
          HIGH_WATERMARK.get_up(),
        );
        if current.0 > prev_max.0 {
          HIGH_WATERMARK.set_down(current.0);
        }
        if current.1 > prev_max.1 {
          HIGH_WATERMARK.set_up(current.1);
        }
      }
  }

  pub(crate) fn next_cycle(&self) {
    self.cycle.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
  }

  pub(crate) fn bits_per_second(&self) -> (u64, u64) {
    (
      self.bytes_per_second.get_down() * 8,
      self.bytes_per_second.get_up() * 8,
    )
  }

  pub(crate) fn shaped_bits_per_second(&self) -> (u64, u64) {
    (
      self.shaped_bytes_per_second.get_down() * 8, 
      self.shaped_bytes_per_second.get_up() * 8
    )
  }

  pub(crate) fn packets_per_second(&self) -> (u64, u64) {
    (
      self.packets_per_second.get_down(),
      self.packets_per_second.get_up(),
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
