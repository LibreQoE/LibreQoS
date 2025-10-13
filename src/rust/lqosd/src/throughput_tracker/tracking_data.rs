use super::{
    RETIRE_AFTER_SECONDS,
    flow_data::{
        ALL_FLOWS, FlowAnalysis, FlowbeeLocalData, RttData, get_flowbee_event_count_and_reset,
    },
    throughput_entry::ThroughputEntry,
};
use crate::{
    shaped_devices_tracker::SHAPED_DEVICES,
    stats::HIGH_WATERMARK,
    throughput_tracker::flow_data::{expire_rtt_flows, flowbee_rtt_map},
};
use fxhash::FxHashMap;
use lqos_bakery::BakeryCommands;
use lqos_bus::TcHandle;
use lqos_config::NetworkJson;
use lqos_queue_tracker::ALL_QUEUE_SUMMARY;
use lqos_sys::{flowbee_data::FlowbeeKey, iterate_flows, throughput_for_each};
use lqos_utils::units::{AtomicDownUp, DownUpOrder};
use lqos_utils::{XdpIpAddress, unix_time::time_since_boot};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::{sync::atomic::AtomicU64, time::Duration};
use tracing::{debug, info, warn};

// Maximum number of flows to track simultaneously
// TODO: This should be made configurable via the config file
const MAX_FLOWS: usize = 1_000_000;

pub const MAX_RETRY_TIMES: usize = 32;

pub struct ThroughputTracker {
    pub(crate) cycle: AtomicU64,
    pub(crate) raw_data: Mutex<HashMap<XdpIpAddress, ThroughputEntry>>,
    pub(crate) bytes_per_second: AtomicDownUp,
    pub(crate) packets_per_second: AtomicDownUp,
    pub(crate) tcp_packets_per_second: AtomicDownUp,
    pub(crate) udp_packets_per_second: AtomicDownUp,
    pub(crate) icmp_packets_per_second: AtomicDownUp,
    pub(crate) shaped_bytes_per_second: AtomicDownUp,
}

impl ThroughputTracker {
    pub(crate) fn new() -> Self {
        // The capacity used to be taken from MAX_TRACKED_IPs, but
        // that's quite wasteful for smaller systems. So we're starting
        // small and allowing vector growth. That will slow down the
        // first few cycles, but it should be fine after that.
        Self {
            cycle: AtomicU64::new(RETIRE_AFTER_SECONDS),
            raw_data: Mutex::default(),
            bytes_per_second: AtomicDownUp::zeroed(),
            packets_per_second: AtomicDownUp::zeroed(),
            tcp_packets_per_second: AtomicDownUp::zeroed(),
            udp_packets_per_second: AtomicDownUp::zeroed(),
            icmp_packets_per_second: AtomicDownUp::zeroed(),
            shaped_bytes_per_second: AtomicDownUp::zeroed(),
        }
    }

    pub(crate) fn copy_previous_and_reset_rtt(&self) {
        // Copy previous byte/packet numbers and reset RTT data
        let self_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);
        let mut raw_data = self.raw_data.lock();
        raw_data.iter_mut().for_each(|(_k, v)| {
            if v.first_cycle < self_cycle {
                v.bytes_per_second = v.bytes.checked_sub_or_zero(v.prev_bytes);
                v.packets_per_second = v.packets.checked_sub_or_zero(v.prev_packets);
            }
            v.prev_bytes = v.bytes;
            v.prev_packets = v.packets;
            v.prev_tcp_packets = v.tcp_packets;
            v.prev_udp_packets = v.udp_packets;
            v.prev_icmp_packets = v.icmp_packets;

            // Roll out stale RTT data
            if self_cycle > RETIRE_AFTER_SECONDS
                && v.last_fresh_rtt_data_cycle < self_cycle - RETIRE_AFTER_SECONDS
            {
                v.recent_rtt_data = [RttData::from_nanos(0); 60];
            }
        });
    }

    fn lookup_circuit_id(xdp_ip: &XdpIpAddress) -> (Option<String>, Option<i64>) {
        let mut circuit_id = None;
        let mut circuit_hash = None;
        let lookup = xdp_ip.as_ipv6();
        let cfg = SHAPED_DEVICES.load();
        if let Some((_, id)) = cfg.trie.longest_match(lookup) {
            circuit_id = Some(cfg.devices[*id].circuit_id.clone());
            circuit_hash = Some(cfg.devices[*id].circuit_hash);
        }
        //println!("{lookup:?} Found circuit_id: {circuit_id:?}");
        (circuit_id, circuit_hash)
    }

    pub(crate) fn get_node_name_for_circuit_id(circuit_id: Option<String>) -> Option<String> {
        if let Some(circuit_id) = circuit_id {
            let shaped = SHAPED_DEVICES.load();
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
        lock: &NetworkJson,
    ) -> Option<Vec<usize>> {
        if let Some(parent) = Self::get_node_name_for_circuit_id(circuit_id) {
            //let lock = crate::shaped_devices_tracker::NETWORK_JSON.read().unwrap();
            lock.get_parents_for_circuit_id(&parent)
        } else {
            None
        }
    }

    pub(crate) fn refresh_circuit_ids(&self, lock: &NetworkJson) {
        let mut raw_data = self.raw_data.lock();
        raw_data.iter_mut().for_each(|(key, data)| {
            let (circuit_id, circuit_hash) = Self::lookup_circuit_id(key);
            data.circuit_id = circuit_id;
            data.circuit_hash = circuit_hash;
            data.network_json_parents = Self::lookup_network_parents(data.circuit_id.clone(), lock);
        });
    }

    pub(crate) fn apply_new_throughput_counters(
        &self,
        net_json_calc: &mut NetworkJson,
        bakery_sender: crossbeam_channel::Sender<BakeryCommands>,
    ) {
        let mut changed_circuits = HashSet::new();

        let self_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);
        let mut raw_data = self.raw_data.lock();
        throughput_for_each(&mut |xdp_ip, counts| {
            if let Some(entry) = raw_data.get_mut(xdp_ip) {
                // Zero the counter, we have to do a per-CPU sum
                entry.bytes = DownUpOrder::zeroed();
                entry.packets = DownUpOrder::zeroed();
                entry.tcp_packets = DownUpOrder::zeroed();
                entry.udp_packets = DownUpOrder::zeroed();
                entry.icmp_packets = DownUpOrder::zeroed();
                // Sum the counts across CPUs (it's a per-CPU map)
                for c in counts {
                    entry
                        .bytes
                        .checked_add_direct(c.download_bytes, c.upload_bytes);
                    entry
                        .packets
                        .checked_add_direct(c.download_packets, c.upload_packets);
                    entry
                        .tcp_packets
                        .checked_add_direct(c.tcp_download_packets, c.tcp_upload_packets);
                    entry
                        .udp_packets
                        .checked_add_direct(c.udp_download_packets, c.udp_upload_packets);
                    entry
                        .icmp_packets
                        .checked_add_direct(c.icmp_download_packets, c.icmp_upload_packets);
                    if c.tc_handle != 0 {
                        entry.tc_handle = TcHandle::from_u32(c.tc_handle);
                    }
                    entry.last_seen = u64::max(entry.last_seen, c.last_seen);
                }
                if entry.packets != entry.prev_packets {
                    entry.most_recent_cycle = self_cycle;
                    // Call to Bakery Update for existing traffic
                    if let Some(circuit_hash) = entry.circuit_hash {
                        changed_circuits.insert(circuit_hash);
                    }

                    if let Some(parents) = &entry.network_json_parents {
                        net_json_calc.add_throughput_cycle(
                            parents,
                            (
                                entry.bytes.down.saturating_sub(entry.prev_bytes.down),
                                entry.bytes.up.saturating_sub(entry.prev_bytes.up),
                            ),
                            (
                                entry.packets.down.saturating_sub(entry.prev_packets.down),
                                entry.packets.up.saturating_sub(entry.prev_packets.up),
                            ),
                            (
                                entry
                                    .tcp_packets
                                    .down
                                    .saturating_sub(entry.prev_tcp_packets.down),
                                entry
                                    .tcp_packets
                                    .up
                                    .saturating_sub(entry.prev_tcp_packets.up),
                            ),
                            (
                                entry
                                    .udp_packets
                                    .down
                                    .saturating_sub(entry.prev_udp_packets.down),
                                entry
                                    .udp_packets
                                    .up
                                    .saturating_sub(entry.prev_udp_packets.up),
                            ),
                            (
                                entry
                                    .icmp_packets
                                    .down
                                    .saturating_sub(entry.prev_icmp_packets.down),
                                entry
                                    .icmp_packets
                                    .up
                                    .saturating_sub(entry.prev_icmp_packets.up),
                            ),
                        );
                    }
                }
            } else {
                let (circuit_id, circuit_hash) = Self::lookup_circuit_id(xdp_ip);
                // Call the Bakery Queue Creation for new circuits
                if let Some(circuit_hash) = circuit_hash {
                    if let Ok(config) = lqos_config::load_config() {
                        if config.queues.lazy_queues.is_some() {
                            let mut add = true;

                            if config.queues.lazy_threshold_bytes.is_some() {
                                let threshold = config.queues.lazy_threshold_bytes.unwrap_or(0);
                                let total_bytes: u64 = counts
                                    .iter()
                                    .map(|c| c.download_bytes + c.upload_bytes)
                                    .sum();
                                if total_bytes < threshold {
                                    add = false;
                                }
                            }

                            if add {
                                changed_circuits.insert(circuit_hash);
                            }
                        }
                    }
                }
                let mut entry = ThroughputEntry {
                    circuit_id: circuit_id.clone(),
                    circuit_hash,
                    network_json_parents: Self::lookup_network_parents(circuit_id, net_json_calc),
                    first_cycle: self_cycle,
                    most_recent_cycle: 0,
                    bytes: DownUpOrder::zeroed(),
                    packets: DownUpOrder::zeroed(),
                    prev_bytes: DownUpOrder::zeroed(),
                    prev_packets: DownUpOrder::zeroed(),
                    bytes_per_second: DownUpOrder::zeroed(),
                    packets_per_second: DownUpOrder::zeroed(),
                    tcp_packets: DownUpOrder::zeroed(),
                    udp_packets: DownUpOrder::zeroed(),
                    icmp_packets: DownUpOrder::zeroed(),
                    prev_tcp_packets: DownUpOrder::zeroed(),
                    prev_udp_packets: DownUpOrder::zeroed(),
                    prev_icmp_packets: DownUpOrder::zeroed(),
                    tc_handle: TcHandle::zero(),
                    recent_rtt_data: [RttData::from_nanos(0); 60],
                    last_fresh_rtt_data_cycle: 0,
                    last_seen: 0,
                    tcp_retransmits: DownUpOrder::zeroed(),
                    prev_tcp_retransmits: DownUpOrder::zeroed(),
                };
                for c in counts {
                    entry
                        .bytes
                        .checked_add_direct(c.download_bytes, c.upload_bytes);
                    entry
                        .packets
                        .checked_add_direct(c.download_packets, c.upload_packets);
                    entry
                        .tcp_packets
                        .checked_add_direct(c.tcp_download_packets, c.tcp_upload_packets);
                    entry
                        .udp_packets
                        .checked_add_direct(c.udp_download_packets, c.udp_upload_packets);
                    entry
                        .icmp_packets
                        .checked_add_direct(c.icmp_download_packets, c.icmp_upload_packets);
                    if c.tc_handle != 0 {
                        entry.tc_handle = TcHandle::from_u32(c.tc_handle);
                    }
                    entry.last_seen = u64::max(entry.last_seen, c.last_seen);
                }
                raw_data.insert(*xdp_ip, entry);
            }
        });

        if !changed_circuits.is_empty() {
            if let Err(e) = bakery_sender.send(BakeryCommands::OnCircuitActivity {
                circuit_ids: changed_circuits,
            }) {
                warn!("Failed to send BakeryCommands::OnCircuitActivity: {:?}", e);
            }
        }
        if let Err(e) = bakery_sender.send(BakeryCommands::Tick) {
            warn!("Failed to send BakeryCommands::Tick: {:?}", e);
        }
    }

    pub(crate) fn apply_queue_stats(&self, net_json_calc: &mut NetworkJson) {
        // Apply totals
        ALL_QUEUE_SUMMARY.calculate_total_queue_stats();

        // Iterate through the queue data and find the matching circuit_id
        let raw_data = self.raw_data.lock();
        ALL_QUEUE_SUMMARY.iterate_queues(|circuit_hash, drops, marks| {
            if let Some((_k, entry)) = raw_data.iter().find(|(_k, v)| match v.circuit_hash {
                Some(ref id) => *id == circuit_hash,
                None => false,
            }) {
                // Find the net_json parents
                if let Some(parents) = &entry.network_json_parents {
                    // Send it upstream
                    net_json_calc.add_queue_cycle(parents, marks, drops);
                }
            }
        });
    }

    pub(crate) fn apply_flow_data(
        &self,
        timeout_seconds: u64,
        _netflow_enabled: bool,
        sender: crossbeam_channel::Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
        net_json_calc: &mut NetworkJson,
        rtt_circuit_tracker: &mut FxHashMap<XdpIpAddress, [Vec<RttData>; 2]>,
        tcp_retries: &mut FxHashMap<XdpIpAddress, DownUpOrder<u64>>,
        expired_keys: &mut Vec<FlowbeeKey>,
    ) {
        //log::debug!("Flowbee events this second: {}", get_flowbee_event_count_and_reset());
        let self_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);

        if let Ok(now) = time_since_boot() {
            let rtt_samples = flowbee_rtt_map();
            get_flowbee_event_count_and_reset();
            let since_boot = Duration::from(now);
            let expire = since_boot
                .saturating_sub(Duration::from_secs(timeout_seconds))
                .as_nanos() as u64;

            let mut all_flows_lock = ALL_FLOWS.lock();
            let mut raw_data = self.raw_data.lock();

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
                    if let Some(this_flow) = all_flows_lock.flow_data.get_mut(&key) {
                        // If retransmits have changed, add the time to the retry list
                        if data.tcp_retransmits.down != this_flow.0.tcp_retransmits.down {
                            if this_flow.0.retry_times_down.is_none() {
                                this_flow.0.retry_times_down = Some((0, [0; MAX_RETRY_TIMES]));
                            }
                            if let Some(retry_times) = &mut this_flow.0.retry_times_down {
                                retry_times.1[retry_times.0] = data.last_seen;
                                retry_times.0 += 1;
                                retry_times.0 %= MAX_RETRY_TIMES;
                            }
                        }
                        if data.tcp_retransmits.up != this_flow.0.tcp_retransmits.up {
                            if this_flow.0.retry_times_up.is_none() {
                                this_flow.0.retry_times_up = Some((0, [0; MAX_RETRY_TIMES]));
                            }
                            if let Some(retry_times) = &mut this_flow.0.retry_times_up {
                                retry_times.1[retry_times.0] = data.last_seen;
                                retry_times.0 += 1;
                                retry_times.0 %= MAX_RETRY_TIMES;
                            }
                        }

                        //let change_since_last_time = data.bytes_sent.checked_sub_or_zero(this_flow.0.bytes_sent);
                        //this_flow.0.throughput_buffer.push(change_since_last_time);
                        //println!("{change_since_last_time:?}");

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
                        // Check if we've hit the flow limit
                        if all_flows_lock.flow_data.len() >= MAX_FLOWS {
                            // Log warning once per second to avoid spam
                            static LAST_WARNING: std::sync::atomic::AtomicU64 =
                                std::sync::atomic::AtomicU64::new(0);
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            let last = LAST_WARNING.load(std::sync::atomic::Ordering::Relaxed);
                            if now > last {
                                warn!("Flow limit of {} reached, dropping new flow", MAX_FLOWS);
                                LAST_WARNING.store(now, std::sync::atomic::Ordering::Relaxed);
                            }
                        } else {
                            // Insert it into the map
                            let flow_analysis = FlowAnalysis::new(&key);
                            all_flows_lock
                                .flow_data
                                .insert(key.clone(), (data.into(), flow_analysis));
                        }
                    }

                    // TCP - we have RTT data? 6 is TCP
                    if key.ip_protocol == 6
                        && data.end_status == 0
                        && raw_data.contains_key(&key.local_ip)
                    {
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
                                rtt_circuit_tracker
                                    .insert(key.local_ip, [vec![rtt[0]], vec![rtt[1]]]);
                            }
                        }

                        // TCP Retries
                        if let Some(retries) = tcp_retries.get_mut(&key.local_ip) {
                            retries.down += data.tcp_retransmits.down as u64;
                            retries.up += data.tcp_retransmits.up as u64;
                        } else {
                            tcp_retries.insert(
                                key.local_ip,
                                DownUpOrder::new(
                                    data.tcp_retransmits.down as u64,
                                    data.tcp_retransmits.up as u64,
                                ),
                            );
                        }
                    }
                    if data.end_status != 0 {
                        // The flow has ended. We need to remove it from the map.
                        expired_keys.push(key.clone());
                    }
                }
            }); // End flow iterator

            // Merge in the per-flow RTT data into the per-circuit tracker
            for (local_ip, rtt_data) in rtt_circuit_tracker {
                let mut rtts = rtt_data[0]
                    .iter()
                    .filter(|r| r.as_nanos() > 0)
                    .collect::<Vec<_>>();
                rtts.extend(rtt_data[1].iter().filter(|r| r.as_nanos() > 0));
                if !rtts.is_empty() {
                    rtts.sort();
                    let median = rtts[rtts.len() / 2];
                    if let Some(tracker) = raw_data.get_mut(&local_ip) {
                        // Only apply if the flow has achieved 1 Mbps or more
                        if tracker.bytes_per_second.sum_exceeds(125_000) {
                            // Shift left
                            for i in 1..60 {
                                tracker.recent_rtt_data[i] = tracker.recent_rtt_data[i - 1];
                            }
                            tracker.recent_rtt_data[0] = *median;
                            tracker.last_fresh_rtt_data_cycle = self_cycle;
                            if let Some(parents) = &tracker.network_json_parents {
                                if let Some(rtt) = tracker.median_latency() {
                                    net_json_calc.add_rtt_cycle(parents, rtt);
                                }
                            }
                        }
                    }
                }
            }

            // Merge in the TCP retries
            // Reset all entries in the tracker to 0
            for (_k, circuit) in raw_data.iter_mut() {
                circuit.tcp_retransmits = DownUpOrder::zeroed();
            }
            // Apply the new ones
            for (local_ip, retries) in tcp_retries {
                if let Some(tracker) = raw_data.get_mut(&local_ip) {
                    tracker.tcp_retransmits.down = retries
                        .down
                        .saturating_sub(tracker.prev_tcp_retransmits.down);
                    tracker.tcp_retransmits.up =
                        retries.up.saturating_sub(tracker.prev_tcp_retransmits.up);
                    tracker.prev_tcp_retransmits.down = retries.down;
                    tracker.prev_tcp_retransmits.up = retries.up;

                    // Send it upstream
                    if let Some(parents) = &tracker.network_json_parents {
                        net_json_calc.add_retransmit_cycle(parents, tracker.tcp_retransmits);
                    }
                }
            }

            // Key Expiration
            if !expired_keys.is_empty() {
                for key in expired_keys.iter() {
                    // Send it off to netperf for analysis if we are supporting doing so.
                    if let Some(d) = all_flows_lock.flow_data.remove(&key) {
                        let _ = sender.send((key.clone(), (d.0.clone(), d.1.clone())));
                    }
                }

                let ret = lqos_sys::end_flows(expired_keys);
                if let Err(e) = ret {
                    warn!("Failed to end flows: {:?}", e);
                }
            }

            // Cleaning run
            all_flows_lock
                .flow_data
                .retain(|_k, v| v.0.last_seen >= expire);
            all_flows_lock.flow_data.shrink_to_fit();
            expire_rtt_flows();
        }
    }

    pub(crate) fn update_totals(&self) {
        let current_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);
        self.bytes_per_second.set_to_zero();
        self.packets_per_second.set_to_zero();
        self.tcp_packets_per_second.set_to_zero();
        self.udp_packets_per_second.set_to_zero();
        self.icmp_packets_per_second.set_to_zero();
        self.shaped_bytes_per_second.set_to_zero();
        let raw_data = self.raw_data.lock();
        raw_data
            .iter()
            .filter(|(_k, v)| {
                v.most_recent_cycle == current_cycle && v.first_cycle + 2 < current_cycle
            })
            .map(|(_k, v)| {
                (
                    v.bytes.down.saturating_sub(v.prev_bytes.down),
                    v.bytes.up.saturating_sub(v.prev_bytes.up),
                    v.packets.down.saturating_sub(v.prev_packets.down),
                    v.packets.up.saturating_sub(v.prev_packets.up),
                    v.tcp_packets.down.saturating_sub(v.prev_tcp_packets.down),
                    v.tcp_packets.up.saturating_sub(v.prev_tcp_packets.up),
                    v.udp_packets.down.saturating_sub(v.prev_udp_packets.down),
                    v.udp_packets.up.saturating_sub(v.prev_udp_packets.up),
                    v.icmp_packets.down.saturating_sub(v.prev_icmp_packets.down),
                    v.icmp_packets.up.saturating_sub(v.prev_icmp_packets.up),
                    v.tc_handle.as_u32() > 0,
                )
            })
            .for_each(
                |(
                    bytes_down,
                    bytes_up,
                    packets_down,
                    packets_up,
                    tcp_down,
                    tcp_up,
                    udp_down,
                    udp_up,
                    icmp_down,
                    icmp_up,
                    shaped,
                )| {
                    self.bytes_per_second
                        .checked_add_tuple((bytes_down, bytes_up));
                    self.packets_per_second
                        .checked_add_tuple((packets_down, packets_up));
                    self.tcp_packets_per_second
                        .checked_add_tuple((tcp_down, tcp_up));
                    self.udp_packets_per_second
                        .checked_add_tuple((udp_down, udp_up));
                    self.icmp_packets_per_second
                        .checked_add_tuple((icmp_down, icmp_up));
                    if shaped {
                        self.shaped_bytes_per_second
                            .checked_add_tuple((bytes_down, bytes_up));
                    }
                },
            );

        let current = self.bits_per_second();
        if current.both_less_than(100000000000) {
            let prev_max = (HIGH_WATERMARK.get_down(), HIGH_WATERMARK.get_up());
            if current.down > prev_max.0 {
                HIGH_WATERMARK.set_down(current.down);
            }
            if current.up > prev_max.1 {
                HIGH_WATERMARK.set_up(current.up);
            }
        }
    }

    pub(crate) fn next_cycle(&self) {
        self.cycle
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Cleanup
        if let Ok(now) = time_since_boot() {
            let since_boot = Duration::from(now);
            let timeout_seconds = 5 * 60; // 5 minutes
            let expire = since_boot
                .saturating_sub(Duration::from_secs(timeout_seconds))
                .as_nanos() as u64;
            let mut keys_to_expire = Vec::new();
            let mut raw_data = self.raw_data.lock();
            raw_data.retain(|k, v| {
                let keep_it = v.last_seen >= expire && v.last_seen > 0;
                if !keep_it {
                    debug!("Removing {:?} from tracking", k);
                    keys_to_expire.push(k.clone());
                }
                keep_it
            });
            raw_data.shrink_to_fit();
            if let Err(e) = lqos_sys::expire_throughput(keys_to_expire) {
                warn!("Failed to expire throughput: {:?}", e);
            }
        }
    }

    pub(crate) fn bits_per_second(&self) -> DownUpOrder<u64> {
        self.bytes_per_second.as_down_up().to_bits_from_bytes()
    }

    pub(crate) fn shaped_bits_per_second(&self) -> DownUpOrder<u64> {
        self.shaped_bytes_per_second
            .as_down_up()
            .to_bits_from_bytes()
    }

    pub(crate) fn packets_per_second(&self) -> DownUpOrder<u64> {
        self.packets_per_second.as_down_up()
    }

    pub(crate) fn tcp_packets_per_second(&self) -> DownUpOrder<u64> {
        self.tcp_packets_per_second.as_down_up()
    }

    pub(crate) fn udp_packets_per_second(&self) -> DownUpOrder<u64> {
        self.udp_packets_per_second.as_down_up()
    }

    pub(crate) fn icmp_packets_per_second(&self) -> DownUpOrder<u64> {
        self.icmp_packets_per_second.as_down_up()
    }

    #[allow(dead_code)]
    pub(crate) fn dump(&self) {
        let raw_data = self.raw_data.lock();
        for (k, v) in raw_data.iter() {
            let ip = k.as_ip();
            info!("{:<34}{:?}", ip, v.tc_handle);
        }
    }
}
