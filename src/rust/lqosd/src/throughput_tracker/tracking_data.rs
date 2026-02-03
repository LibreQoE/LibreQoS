use super::{
    RETIRE_AFTER_SECONDS,
    flow_data::{
        ALL_FLOWS, AsnAggregate, FlowAnalysis, FlowbeeLocalData, RttBuffer, RttData,
        get_flowbee_event_count_and_reset, update_asn_heatmaps,
    },
    throughput_entry::ThroughputEntry,
};
use crate::throughput_tracker::CIRCUIT_RTT_BUFFERS;
use crate::{
    shaped_devices_tracker::SHAPED_DEVICES,
    stats::HIGH_WATERMARK,
    throughput_tracker::flow_data::{expire_rtt_flows, flowbee_rtt_map, FlowbeeEffectiveDirection},
};
use fxhash::FxHashMap;
use lqos_bakery::BakeryCommands;
use lqos_bus::TcHandle;
use lqos_config::NetworkJson;
use lqos_queue_tracker::ALL_QUEUE_SUMMARY;
use lqos_sys::{flowbee_data::FlowbeeKey, iterate_flows, throughput_for_each};
use lqos_utils::{XdpIpAddress, unix_time::time_since_boot};
use lqos_utils::{
    temporal_heatmap::TemporalHeatmap,
    qoq_heatmap::TemporalQoqHeatmap,
    rtt::RttBucket,
    units::{AtomicDownUp, DownUpOrder},
};
use lqos_utils::qoo::{LossMeasurement, QOQ_UNKNOWN, QoqScores, compute_qoq_scores};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::{sync::atomic::AtomicU64, time::Duration};
use tracing::{debug, info, warn};

// Maximum number of flows to track simultaneously
// TODO: This should be made configurable via the config file
const MAX_FLOWS: usize = 1_000_000;

pub const MAX_RETRY_TIMES: usize = 128;
const MIN_QOO_FLOW_BYTES: u64 = 1_000_000;

pub struct ThroughputTracker {
    pub(crate) cycle: AtomicU64,
    pub(crate) raw_data: Mutex<HashMap<XdpIpAddress, ThroughputEntry>>,
    pub(crate) bytes_per_second: AtomicDownUp,
    pub(crate) packets_per_second: AtomicDownUp,
    pub(crate) tcp_packets_per_second: AtomicDownUp,
    pub(crate) udp_packets_per_second: AtomicDownUp,
    pub(crate) icmp_packets_per_second: AtomicDownUp,
    pub(crate) shaped_bytes_per_second: AtomicDownUp,
    pub(crate) circuit_heatmaps: Mutex<FxHashMap<i64, TemporalHeatmap>>,
    pub(crate) circuit_qoq_heatmaps: Mutex<FxHashMap<i64, TemporalQoqHeatmap>>,
    pub(crate) global_heatmap: Mutex<TemporalHeatmap>,
    pub(crate) global_qoq_heatmap: Mutex<TemporalQoqHeatmap>,
}

#[derive(Default)]
struct CircuitHeatmapAggregate {
    download_bytes: u64,
    upload_bytes: u64,
    tcp_retransmits: DownUpOrder<u64>,
    tcp_packets: DownUpOrder<u64>,
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
            circuit_heatmaps: Mutex::default(),
            circuit_qoq_heatmaps: Mutex::default(),
            global_heatmap: Mutex::new(TemporalHeatmap::new()),
            global_qoq_heatmap: Mutex::new(TemporalQoqHeatmap::new()),
        }
    }

    pub(crate) fn record_circuit_heatmaps(&self) {
        let Ok(config) = lqos_config::load_config() else {
            return;
        };
        let qoo_profile = lqos_config::active_qoo_profile().ok();
        let global_down_mbps = config.queues.downlink_bandwidth_mbps as f32;
        let global_up_mbps = config.queues.uplink_bandwidth_mbps as f32;

        if !config.enable_circuit_heatmaps {
            self.circuit_heatmaps.lock().clear();
            self.circuit_qoq_heatmaps.lock().clear();
            *self.global_heatmap.lock() = TemporalHeatmap::new();
            *self.global_qoq_heatmap.lock() = TemporalQoqHeatmap::new();
            return;
        }

        let shaped_devices = SHAPED_DEVICES.load();
        let mut capacity_lookup: FxHashMap<i64, (f32, f32)> = FxHashMap::default();
        capacity_lookup.reserve(shaped_devices.devices.len());
        shaped_devices.devices.iter().for_each(|device| {
            let entry = capacity_lookup
                .entry(device.circuit_hash)
                .or_insert((device.download_max_mbps, device.upload_max_mbps));
            if device.download_max_mbps > entry.0 {
                entry.0 = device.download_max_mbps;
            }
            if device.upload_max_mbps > entry.1 {
                entry.1 = device.upload_max_mbps;
            }
        });

        let mut aggregates: FxHashMap<i64, CircuitHeatmapAggregate> = FxHashMap::default();
        let mut total_download_bytes: u64 = 0;
        let mut total_upload_bytes: u64 = 0;
        let mut total_retransmits: DownUpOrder<u64> = DownUpOrder::zeroed();
        let mut total_tcp_packets: DownUpOrder<u64> = DownUpOrder::zeroed();
        let circuit_rtt_snapshot = CIRCUIT_RTT_BUFFERS.load();
        {
            let raw_data = self.raw_data.lock();
            for entry in raw_data.values() {
                let circuit_hash = if let Some(circuit_hash) = entry.circuit_hash {
                    circuit_hash
                } else {
                    continue;
                };

                let download_delta = entry.bytes.down.saturating_sub(entry.prev_bytes.down);
                let upload_delta = entry.bytes.up.saturating_sub(entry.prev_bytes.up);
                total_download_bytes = total_download_bytes.saturating_add(download_delta);
                total_upload_bytes = total_upload_bytes.saturating_add(upload_delta);
                total_tcp_packets.down = total_tcp_packets.down.saturating_add(
                    entry
                        .tcp_packets
                        .down
                        .saturating_sub(entry.prev_tcp_packets.down),
                );
                total_tcp_packets.up = total_tcp_packets.up.saturating_add(
                    entry
                        .tcp_packets
                        .up
                        .saturating_sub(entry.prev_tcp_packets.up),
                );
                total_retransmits.down = total_retransmits
                    .down
                    .saturating_add(entry.tcp_retransmits.down);
                total_retransmits.up = total_retransmits
                    .up
                    .saturating_add(entry.tcp_retransmits.up);

                let agg = aggregates
                    .entry(circuit_hash)
                    .or_insert_with(CircuitHeatmapAggregate::default);
                agg.download_bytes = agg.download_bytes.saturating_add(download_delta);
                agg.upload_bytes = agg.upload_bytes.saturating_add(upload_delta);
                agg.tcp_packets.down = agg.tcp_packets.down.saturating_add(
                    entry
                        .tcp_packets
                        .down
                        .saturating_sub(entry.prev_tcp_packets.down),
                );
                agg.tcp_packets.up = agg.tcp_packets.up.saturating_add(
                    entry
                        .tcp_packets
                        .up
                        .saturating_sub(entry.prev_tcp_packets.up),
                );
                agg.tcp_retransmits.down = agg
                    .tcp_retransmits
                    .down
                    .saturating_add(entry.tcp_retransmits.down);
                agg.tcp_retransmits.up = agg
                    .tcp_retransmits
                    .up
                    .saturating_add(entry.tcp_retransmits.up);
            }
        }

        let mut heatmaps = self.circuit_heatmaps.lock();
        heatmaps.retain(|circuit_hash, _| capacity_lookup.contains_key(circuit_hash));
        let mut qoq_heatmaps = self.circuit_qoq_heatmaps.lock();
        qoq_heatmaps.retain(|circuit_hash, _| capacity_lookup.contains_key(circuit_hash));
        let empty_rtt = RttBuffer::default();

        for (circuit_hash, aggregate) in aggregates {
            let (max_down_mbps, max_up_mbps) = capacity_lookup
                .get(&circuit_hash)
                .copied()
                .unwrap_or((0.0, 0.0));

            let download_util =
                utilization_percent(aggregate.download_bytes, max_down_mbps).unwrap_or(0.0);
            let upload_util =
                utilization_percent(aggregate.upload_bytes, max_up_mbps).unwrap_or(0.0);
            let rtt_p50_down = circuit_rtt_snapshot
                .get(&circuit_hash)
                .and_then(|rtt| rtt.percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50))
                .map(|rtt| rtt.as_millis() as f32);
            let rtt_p50_up = circuit_rtt_snapshot
                .get(&circuit_hash)
                .and_then(|rtt| rtt.percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50))
                .map(|rtt| rtt.as_millis() as f32);
            let rtt_p90_down = circuit_rtt_snapshot
                .get(&circuit_hash)
                .and_then(|rtt| rtt.percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 90))
                .map(|rtt| rtt.as_millis() as f32);
            let rtt_p90_up = circuit_rtt_snapshot
                .get(&circuit_hash)
                .and_then(|rtt| rtt.percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 90))
                .map(|rtt| rtt.as_millis() as f32);
            let retransmit_down =
                retransmit_percent(aggregate.tcp_retransmits.down, aggregate.tcp_packets.down);
            let retransmit_up =
                retransmit_percent(aggregate.tcp_retransmits.up, aggregate.tcp_packets.up);

            let heatmap = heatmaps
                .entry(circuit_hash)
                .or_insert_with(TemporalHeatmap::new);
            heatmap.add_sample(
                download_util,
                upload_util,
                rtt_p50_down,
                rtt_p50_up,
                rtt_p90_down,
                rtt_p90_up,
                retransmit_down,
                retransmit_up,
            );

            let rtt = circuit_rtt_snapshot.get(&circuit_hash).unwrap_or(&empty_rtt);
            let loss_download =
                tcp_retransmit_loss_proxy(aggregate.tcp_retransmits.down, aggregate.tcp_packets.down);
            let loss_upload =
                tcp_retransmit_loss_proxy(aggregate.tcp_retransmits.up, aggregate.tcp_packets.up);
            let scores = if let Some(profile) = qoo_profile.as_ref() {
                compute_qoq_scores(
                    profile.as_ref(),
                    rtt,
                    loss_download,
                    loss_upload,
                )
            } else {
                QoqScores::default()
            };
            let qoq_heatmap = qoq_heatmaps
                .entry(circuit_hash)
                .or_insert_with(TemporalQoqHeatmap::new);
            qoq_heatmap.add_sample(
                scores.download_total_f32(),
                scores.upload_total_f32(),
            );
        }

        let mut global_rtt_buffer = RttBuffer::default();
        for rtt in circuit_rtt_snapshot.values() {
            global_rtt_buffer.accumulate(rtt);
        }
        let global_rtt_p50_down = global_rtt_buffer
            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50)
            .map(|rtt| rtt.as_millis() as f32);
        let global_rtt_p50_up = global_rtt_buffer
            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50)
            .map(|rtt| rtt.as_millis() as f32);
        let global_rtt_p90_down = global_rtt_buffer
            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 90)
            .map(|rtt| rtt.as_millis() as f32);
        let global_rtt_p90_up = global_rtt_buffer
            .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 90)
            .map(|rtt| rtt.as_millis() as f32);

        let mut global_heatmap = self.global_heatmap.lock();
        let global_retransmit_down =
            retransmit_percent(total_retransmits.down, total_tcp_packets.down);
        let global_retransmit_up = retransmit_percent(total_retransmits.up, total_tcp_packets.up);
        global_heatmap.add_sample(
            utilization_percent(total_download_bytes, global_down_mbps).unwrap_or(0.0),
            utilization_percent(total_upload_bytes, global_up_mbps).unwrap_or(0.0),
            global_rtt_p50_down,
            global_rtt_p50_up,
            global_rtt_p90_down,
            global_rtt_p90_up,
            global_retransmit_down,
            global_retransmit_up,
        );

        // QoO is derived from RTT histogram + retransmit proxy.
        let loss_download =
            tcp_retransmit_loss_proxy(total_retransmits.down, total_tcp_packets.down);
        let loss_upload = tcp_retransmit_loss_proxy(total_retransmits.up, total_tcp_packets.up);

        let scores = if let Some(profile) = qoo_profile.as_ref() {
            compute_qoq_scores(
                profile.as_ref(),
                &global_rtt_buffer,
                loss_download,
                loss_upload,
            )
        } else {
            QoqScores::default()
        };

        self.global_qoq_heatmap.lock().add_sample(
            scores.download_total_f32(),
            scores.upload_total_f32(),
        );
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
                v.rtt_buffer.clear();
                v.qoq = QoqScores::default();
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
                    rtt_buffer: RttBuffer::default(),
                    recent_rtt_data: [RttData::from_nanos(0); 60],
                    last_fresh_rtt_data_cycle: 0,
                    last_seen: 0,
                    tcp_retransmits: DownUpOrder::zeroed(),
                    prev_tcp_retransmits: DownUpOrder::zeroed(),
                    qoq: QoqScores::default(),
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
        rtt_circuit_tracker: &mut FxHashMap<XdpIpAddress, RttBuffer>,
        rtt_by_circuit: &mut FxHashMap<i64, RttBuffer>,
        tcp_retries: &mut FxHashMap<XdpIpAddress, DownUpOrder<u64>>,
        expired_keys: &mut Vec<FlowbeeKey>,
    ) {
        //log::debug!("Flowbee events this second: {}", get_flowbee_event_count_and_reset());
        let self_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);
        let enable_asn_heatmaps = lqos_config::load_config()
            .map(|config| config.enable_asn_heatmaps)
            .unwrap_or(true);
        let qoo_profile = lqos_config::active_qoo_profile().ok();
        let mut asn_aggregates: FxHashMap<u32, AsnAggregate> = FxHashMap::default();
        let mut add_asn_sample = |asn: u32,
                                  bytes: DownUpOrder<u64>,
                                  packets: DownUpOrder<u64>,
                                  retransmits: DownUpOrder<u64>,
                                  rtt_ms: Option<f32>| {
            if asn == 0 {
                return;
            }
            let agg = asn_aggregates
                .entry(asn)
                .or_insert_with(AsnAggregate::default);
            agg.bytes.checked_add(bytes);
            agg.packets.checked_add(packets);
            agg.retransmits.checked_add(retransmits);
            if let Some(rtt) = rtt_ms {
                agg.rtts.push(rtt);
            }
        };

        if let Ok(now) = time_since_boot() {
            let mut rtt_samples = flowbee_rtt_map();
            get_flowbee_event_count_and_reset();
            let since_boot = Duration::from(now);
            let expire = since_boot
                .saturating_sub(Duration::from_secs(timeout_seconds))
                .as_nanos() as u64;

            let mut all_flows_lock = ALL_FLOWS.lock();
            let mut raw_data = self.raw_data.lock();

            // Track through all the flows
            iterate_flows(&mut |key, data| {
                let mut rtt_buffer = rtt_samples.remove(key);
                let mut rtt_for_circuit: Option<[RttData; 2]> = None;
                if data.end_status == 3 {
                    // The flow has been handled already and should be ignored.
                    // DO NOT process it again.
                } else if data.last_seen < expire {
                    // This flow has expired but not been handled yet. Add it to the list to be cleaned.
                    expired_keys.push(key.clone());
                } else {
                    // We have a valid flow, so it needs to be tracked
                    if let Some(this_flow) = all_flows_lock.flow_data.get_mut(&key) {
                        let delta_bytes =
                            data.bytes_sent.checked_sub_or_zero(this_flow.0.bytes_sent);
                        let delta_packets = data
                            .packets_sent
                            .checked_sub_or_zero(this_flow.0.packets_sent);
                        let delta_retrans = data
                            .tcp_retransmits
                            .checked_sub_or_zero(this_flow.0.tcp_retransmits);
                        let delta_retrans =
                            DownUpOrder::new(delta_retrans.down as u64, delta_retrans.up as u64);
                        // If retransmits have changed, add the time to the retry list
                        if data.tcp_retransmits.down != this_flow.0.tcp_retransmits.down {
                            this_flow
                                .0
                                .record_tcp_retry_time(FlowbeeEffectiveDirection::Download, data.last_seen);
                        }
                        if data.tcp_retransmits.up != this_flow.0.tcp_retransmits.up {
                            this_flow
                                .0
                                .record_tcp_retry_time(FlowbeeEffectiveDirection::Upload, data.last_seen);
                        }

                        //let change_since_last_time = data.bytes_sent.checked_sub_or_zero(this_flow.0.bytes_sent);
                        //this_flow.0.throughput_buffer.push(change_since_last_time);
                        //println!("{change_since_last_time:?}");

                        this_flow.0.set_last_seen(data.last_seen);
                        this_flow.0.set_bytes_sent(data.bytes_sent);
                        this_flow.0.set_packets_sent(data.packets_sent);
                        this_flow.0.set_rate_estimate_bps(data.rate_estimate_bps);
                        this_flow.0.set_tcp_retransmits(data.tcp_retransmits);
                        this_flow.0.set_end_status(data.end_status);
                        this_flow.0.set_tos(data.tos);
                        this_flow.0.set_flags(data.flags);

                        if let Some(rtt_buffer) = rtt_buffer.take() {
                            // Accumulate histogram data per-device so the device median is
                            // weighted by RTT sample volume (not just per-flow medians).
                            if key.ip_protocol == 6
                                && data.end_status == 0
                                && raw_data.contains_key(&key.local_ip)
                            {
                                let device_rtt =
                                    rtt_circuit_tracker.entry(key.local_ip).or_default();
                                if data.bytes_sent.down >= MIN_QOO_FLOW_BYTES {
                                    device_rtt.accumulate_direction(
                                        &rtt_buffer,
                                        FlowbeeEffectiveDirection::Download,
                                    );
                                }
                                if data.bytes_sent.up >= MIN_QOO_FLOW_BYTES {
                                    device_rtt.accumulate_direction(
                                        &rtt_buffer,
                                        FlowbeeEffectiveDirection::Upload,
                                    );
                                }
                            }

                            this_flow.0.set_rtt_buffer(rtt_buffer);
                        }

                        // Per-flow QoO (stored for UI display).
                        if key.ip_protocol == 6 {
                            if let (Some(profile), Some(tcp_info)) =
                                (qoo_profile.as_ref(), this_flow.0.tcp_info.as_ref())
                            {
                                let loss_download = tcp_retransmit_loss_proxy(
                                    this_flow.0.tcp_retransmits.down as u64,
                                    this_flow.0.packets_sent.down,
                                );
                                let loss_upload = tcp_retransmit_loss_proxy(
                                    this_flow.0.tcp_retransmits.up as u64,
                                    this_flow.0.packets_sent.up,
                                );
                                let scores = compute_qoq_scores(
                                    profile.as_ref(),
                                    &tcp_info.rtt,
                                    loss_download,
                                    loss_upload,
                                );
                                let mut scores = scores;
                                if data.bytes_sent.down < MIN_QOO_FLOW_BYTES {
                                    scores.download_total = QOQ_UNKNOWN;
                                }
                                if data.bytes_sent.up < MIN_QOO_FLOW_BYTES {
                                    scores.upload_total = QOQ_UNKNOWN;
                                }
                                this_flow.0.set_qoq_scores(scores);
                            }
                        }
                        if enable_asn_heatmaps {
                            let flow_rtt = combine_rtt_ms(this_flow.0.get_rtt_array());
                            add_asn_sample(
                                this_flow.1.asn_id.0,
                                delta_bytes,
                                delta_packets,
                                delta_retrans,
                                flow_rtt,
                            );
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
                            let mut flow_summary = FlowbeeLocalData::from_flow(&data, &key);
                            if let Some(rtt_buffer) = rtt_buffer.take() {
                                if key.ip_protocol == 6
                                    && data.end_status == 0
                                    && raw_data.contains_key(&key.local_ip)
                                {
                                    let device_rtt =
                                        rtt_circuit_tracker.entry(key.local_ip).or_default();
                                    if data.bytes_sent.down >= MIN_QOO_FLOW_BYTES {
                                        device_rtt.accumulate_direction(
                                            &rtt_buffer,
                                            FlowbeeEffectiveDirection::Download,
                                        );
                                    }
                                    if data.bytes_sent.up >= MIN_QOO_FLOW_BYTES {
                                        device_rtt.accumulate_direction(
                                            &rtt_buffer,
                                            FlowbeeEffectiveDirection::Upload,
                                        );
                                    }
                                }
                                rtt_for_circuit = Some([
                                    rtt_buffer.median_new_data(FlowbeeEffectiveDirection::Download),
                                    rtt_buffer.median_new_data(FlowbeeEffectiveDirection::Upload),
                                ]);
                                flow_summary.set_rtt_buffer(rtt_buffer);
                            }

                            // Per-flow QoO (stored for UI display).
                            if key.ip_protocol == 6 {
                                if let (Some(profile), Some(tcp_info)) =
                                    (qoo_profile.as_ref(), flow_summary.tcp_info.as_ref())
                                {
                                    let loss_download = tcp_retransmit_loss_proxy(
                                        flow_summary.tcp_retransmits.down as u64,
                                        flow_summary.packets_sent.down,
                                    );
                                    let loss_upload = tcp_retransmit_loss_proxy(
                                        flow_summary.tcp_retransmits.up as u64,
                                        flow_summary.packets_sent.up,
                                    );
                                    let scores = compute_qoq_scores(
                                        profile.as_ref(),
                                        &tcp_info.rtt,
                                        loss_download,
                                        loss_upload,
                                    );
                                    let mut scores = scores;
                                    if data.bytes_sent.down < MIN_QOO_FLOW_BYTES {
                                        scores.download_total = QOQ_UNKNOWN;
                                    }
                                    if data.bytes_sent.up < MIN_QOO_FLOW_BYTES {
                                        scores.upload_total = QOQ_UNKNOWN;
                                    }
                                    flow_summary.set_qoq_scores(scores);
                                }
                            }
                            if enable_asn_heatmaps {
                                let flow_rtt = rtt_for_circuit.and_then(combine_rtt_ms);
                                let delta_retrans = DownUpOrder::new(
                                    data.tcp_retransmits.down as u64,
                                    data.tcp_retransmits.up as u64,
                                );
                                add_asn_sample(
                                    flow_analysis.asn_id.0,
                                    data.bytes_sent,
                                    data.packets_sent,
                                    delta_retrans,
                                    flow_rtt,
                                );
                            }
                            all_flows_lock
                                .flow_data
                                .insert(key.clone(), (flow_summary, flow_analysis));
                        }
                    }

                    // TCP - we have RTT data? 6 is TCP
                    if key.ip_protocol == 6
                        && data.end_status == 0
                        && raw_data.contains_key(&key.local_ip)
                    {
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
            for (local_ip, rtt_buffer) in rtt_circuit_tracker {
                let rtt_buffer = std::mem::take(rtt_buffer);
                let download = rtt_buffer.median_new_data(FlowbeeEffectiveDirection::Download);
                let upload = rtt_buffer.median_new_data(FlowbeeEffectiveDirection::Upload);

                let rtt_median = match (download.as_nanos(), upload.as_nanos()) {
                    (0, 0) => None,
                    (d, 0) => Some(RttData::from_nanos(d)),
                    (0, u) => Some(RttData::from_nanos(u)),
                    (d, u) => Some(RttData::from_nanos(d.saturating_add(u) / 2)),
                };

                if let Some(rtt_median) = rtt_median {
                    if let Some(tracker) = raw_data.get_mut(&local_ip) {
                        // Shift left
                        for i in 1..60 {
                            tracker.recent_rtt_data[i] = tracker.recent_rtt_data[i - 1];
                        }
                        tracker.recent_rtt_data[0] = rtt_median;
                        tracker.last_fresh_rtt_data_cycle = self_cycle;
                        tracker.rtt_buffer = rtt_buffer;
                        if let Some(circuit_hash) = tracker.circuit_hash {
                            rtt_by_circuit
                                .entry(circuit_hash)
                                .or_default()
                                .accumulate(&tracker.rtt_buffer);
                        }
                        if let Some(parents) = &tracker.network_json_parents {
                            net_json_calc.add_rtt_buffer_cycle(parents, &tracker.rtt_buffer);
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

            // Per-device QoO (stored for UI display via `NetworkTreeClients`).
            //
            // NOTE: `compute_qoq_scores` uses the TOTAL RTT histogram bucket, so scores remain
            // meaningful even when the current RTT window has few samples. We only update scores
            // when prerequisites are available; otherwise we keep the last known values so the UI
            // doesn't flap to unknown ("-") on idle seconds.
            if let Some(profile) = qoo_profile.as_ref() {
                for tracker in raw_data.values_mut() {
                    let tcp_packets_delta = tracker.tcp_packets.checked_sub_or_zero(tracker.prev_tcp_packets);
                    let loss_download =
                        tcp_retransmit_loss_proxy(tracker.tcp_retransmits.down, tcp_packets_delta.down);
                    let loss_upload =
                        tcp_retransmit_loss_proxy(tracker.tcp_retransmits.up, tcp_packets_delta.up);
                    let scores = compute_qoq_scores(
                        profile.as_ref(),
                        &tracker.rtt_buffer,
                        loss_download,
                        loss_upload,
                    );
                    if scores.download_total != QOQ_UNKNOWN {
                        tracker.qoq.download_total = scores.download_total;
                    }
                    if scores.upload_total != QOQ_UNKNOWN {
                        tracker.qoq.upload_total = scores.upload_total;
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

        if enable_asn_heatmaps || !asn_aggregates.is_empty() {
            update_asn_heatmaps(asn_aggregates, self_cycle, enable_asn_heatmaps);
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

fn utilization_percent(bytes: u64, max_mbps: f32) -> Option<f32> {
    if max_mbps <= 0.0 {
        return None;
    }
    let bits_per_second = bytes.saturating_mul(8) as f64;
    // Some installations store capacity already in bps; others use Mbps.
    // Heuristically treat very large values as bps to avoid double-scaling.
    let capacity_bps = if max_mbps > 1_000_000.0 {
        max_mbps as f64
    } else {
        max_mbps as f64 * 1_000_000.0
    };
    Some(((bits_per_second / capacity_bps) * 100.0) as f32)
}

fn retransmit_percent(retransmits: u64, packets: u64) -> Option<f32> {
    if retransmits == 0 || packets < 10 {
        return None;
    }
    let value =(retransmits as f32 / packets as f32) * 100.0;
    if value > 50.0 {
        None
    } else {
        Some(value)
    }
}

fn median(values: &mut Vec<f32>) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[mid])
    } else {
        Some((values[mid - 1] + values[mid]) / 2.0)
    }
}

fn combine_rtt_ms(rtts: [RttData; 2]) -> Option<f32> {
    let mut samples = Vec::with_capacity(2);
    if rtts[0].as_nanos() > 0 {
        samples.push(rtts[0].as_millis() as f32);
    }
    if rtts[1].as_nanos() > 0 {
        samples.push(rtts[1].as_millis() as f32);
    }
    median(&mut samples)
}

fn tcp_retransmit_loss_proxy(retransmits: u64, packets: u64) -> Option<LossMeasurement> {
    if packets == 0 {
        return None;
    }

    let retransmit_fraction = (retransmits as f64 / packets as f64).clamp(0.0, 1.0);
    // TCP retransmits are only a weak proxy for loss on a transparent bridge. Treat them as low
    // confidence, even with large sample sizes.
    const TCP_RETRANSMIT_CONFIDENCE_MAX: f64 = 0.05;
    let confidence = (packets as f64 / 10_000.0).clamp(0.0, 1.0) * TCP_RETRANSMIT_CONFIDENCE_MAX;
    Some(LossMeasurement::TcpRetransmitProxy {
        retransmit_fraction,
        confidence,
    })
}
