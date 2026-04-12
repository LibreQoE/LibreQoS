use super::{
    CIRCUIT_REPRESENTATIVE_METRICS, CircuitRepresentativeMetrics, RETIRE_AFTER_SECONDS,
    flow_data::{
        ALL_FLOWS, AsnAggregate, FlowAnalysis, FlowbeeLocalData, RttBuffer, RttData,
        get_flowbee_event_count_and_reset, update_asn_heatmaps,
    },
    throughput_entry::ThroughputEntry,
};
use crate::throughput_tracker::CIRCUIT_RTT_BUFFERS;
use crate::{
    shaped_devices_tracker::{
        SHAPED_DEVICE_HASH_CACHE, SHAPED_DEVICES, shaped_device_from_hashes_or_ip,
    },
    stats::HIGH_WATERMARK,
    throughput_tracker::flow_data::{FlowbeeEffectiveDirection, expire_rtt_flows, flowbee_rtt_map},
};
use fxhash::FxHashMap;
use lqos_bakery::BakeryCommands;
use lqos_bus::TcHandle;
use lqos_config::NetworkJson;
use lqos_queue_tracker::ALL_QUEUE_SUMMARY;
use lqos_sys::{flowbee_data::FlowbeeKey, iterate_flows, throughput_for_each};
use lqos_utils::qoo::{LossMeasurement, QOQ_UNKNOWN, QoqScores, compute_qoq_scores};
use lqos_utils::{XdpIpAddress, unix_time::time_since_boot};
use lqos_utils::{
    qoq_heatmap::TemporalQoqHeatmap,
    rtt::RttBucket,
    temporal_heatmap::TemporalHeatmap,
    units::{AtomicDownUp, DownUpOrder},
};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::{sync::Arc, sync::atomic::AtomicU64, time::Duration};
use tracing::{debug, info, warn};

// Maximum number of flows to track simultaneously
// TODO: This should be made configurable via the config file
const MAX_FLOWS: usize = 1_000_000;

pub const MAX_RETRY_TIMES: usize = 128;
pub(crate) const MIN_QOO_FLOW_BYTES: u64 = 1_000_000;
const REPRESENTATIVE_MIN_FLOW_BYTES: u64 = 128 * 1024;

pub(crate) struct FlowApplyContext<'a> {
    pub(crate) timeout_seconds: u64,
    pub(crate) sender: crossbeam_channel::Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
    pub(crate) net_json_calc: &'a mut NetworkJson,
    pub(crate) rtt_circuit_tracker: &'a mut FxHashMap<XdpIpAddress, RttBuffer>,
    pub(crate) rtt_by_circuit: &'a mut FxHashMap<i64, RttBuffer>,
    pub(crate) tcp_retries: &'a mut FxHashMap<XdpIpAddress, DownUpOrder<u64>>,
    pub(crate) tcp_retry_packets: &'a mut FxHashMap<XdpIpAddress, DownUpOrder<u64>>,
    pub(crate) expired_keys: &'a mut Vec<FlowbeeKey>,
}

pub struct ThroughputTracker {
    pub(crate) cycle: AtomicU64,
    pub(crate) raw_data: Mutex<HashMap<XdpIpAddress, ThroughputEntry>>,
    pub(crate) bytes_per_second: AtomicDownUp,
    pub(crate) actual_bytes_per_second: AtomicDownUp,
    pub(crate) packets_per_second: AtomicDownUp,
    pub(crate) tcp_packets_per_second: AtomicDownUp,
    pub(crate) udp_packets_per_second: AtomicDownUp,
    pub(crate) icmp_packets_per_second: AtomicDownUp,
    pub(crate) shaped_bytes_per_second: AtomicDownUp,
    pub(crate) shaped_actual_bytes_per_second: AtomicDownUp,
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

#[derive(Default)]
struct RepresentativeAsnAggregate {
    total_bps: DownUpOrder<u64>,
    rtt_visible_bps: DownUpOrder<u64>,
    tcp_packets: DownUpOrder<u64>,
    tcp_retransmits: DownUpOrder<u64>,
    rtt: RttBuffer,
}

const REPRESENTATIVE_MAX_ASN_SHARE: f64 = 0.15;

fn representative_weight(total_bps: u64, visible_bps: u64) -> Option<f64> {
    if total_bps == 0 || visible_bps == 0 {
        return None;
    }
    let confidence = visible_bps as f64 / total_bps as f64;
    Some((total_bps as f64).ln_1p() * confidence)
}

fn accumulate_representative_direction(
    bucket: &mut RepresentativeAsnAggregate,
    rtt: &RttBuffer,
    direction: FlowbeeEffectiveDirection,
    rate_estimate_bps: u32,
    bytes_sent: u64,
) {
    if bytes_sent < REPRESENTATIVE_MIN_FLOW_BYTES {
        return;
    }

    bucket.rtt.accumulate_direction(rtt, direction);
    if rtt.percentile(RttBucket::Current, direction, 50).is_none() {
        return;
    }

    match direction {
        FlowbeeEffectiveDirection::Download => {
            bucket.rtt_visible_bps.down = bucket
                .rtt_visible_bps
                .down
                .saturating_add(rate_estimate_bps as u64);
        }
        FlowbeeEffectiveDirection::Upload => {
            bucket.rtt_visible_bps.up = bucket
                .rtt_visible_bps
                .up
                .saturating_add(rate_estimate_bps as u64);
        }
    }
}

fn capped_normalized_weights(raw_weights: &[f64], max_share: f64) -> Vec<f64> {
    if raw_weights.is_empty() {
        return Vec::new();
    }
    let max_share = max_share.clamp(0.0, 1.0);
    if max_share <= 0.0 {
        return vec![0.0; raw_weights.len()];
    }
    if max_share >= 1.0 {
        let total: f64 = raw_weights.iter().copied().sum();
        if total <= 0.0 {
            return vec![0.0; raw_weights.len()];
        }
        return raw_weights.iter().map(|weight| *weight / total).collect();
    }

    let active_count = raw_weights.iter().filter(|weight| **weight > 0.0).count();
    if active_count == 0 {
        return vec![0.0; raw_weights.len()];
    }
    if (active_count as f64) * max_share < 1.0 {
        let total: f64 = raw_weights.iter().copied().sum();
        if total <= 0.0 {
            return vec![0.0; raw_weights.len()];
        }
        return raw_weights.iter().map(|weight| *weight / total).collect();
    }

    let mut normalized = vec![0.0; raw_weights.len()];
    let mut active: Vec<usize> = raw_weights
        .iter()
        .enumerate()
        .filter_map(|(idx, weight)| (*weight > 0.0).then_some(idx))
        .collect();
    let mut remaining_mass = 1.0;
    let mut remaining_weight: f64 = active.iter().map(|idx| raw_weights[*idx]).sum();

    while !active.is_empty() && remaining_mass > 0.0 && remaining_weight > 0.0 {
        let max_raw_for_remaining = max_share * remaining_weight / remaining_mass;
        let mut capped_any = false;
        active.retain(|idx| {
            if raw_weights[*idx] > max_raw_for_remaining {
                normalized[*idx] = max_share;
                remaining_mass -= max_share;
                remaining_weight -= raw_weights[*idx];
                capped_any = true;
                false
            } else {
                true
            }
        });
        if !capped_any {
            for idx in active {
                normalized[idx] = raw_weights[idx] * remaining_mass / remaining_weight;
            }
            break;
        }
    }

    normalized
}

fn weighted_median_u64(values: &mut Vec<(u64, f64)>) -> Option<u64> {
    values.retain(|(_, weight)| *weight > 0.0);
    if values.is_empty() {
        return None;
    }
    values.sort_by_key(|(value, _)| *value);
    let normalized_weights = capped_normalized_weights(
        &values.iter().map(|(_, weight)| *weight).collect::<Vec<_>>(),
        REPRESENTATIVE_MAX_ASN_SHARE,
    );
    let total_weight: f64 = normalized_weights.iter().sum();
    if total_weight <= 0.0 {
        return None;
    }
    let threshold = total_weight / 2.0;
    let mut running = 0.0;
    for ((value, _), weight) in values.iter().zip(normalized_weights.iter()) {
        running += *weight;
        if running >= threshold {
            return Some(*value);
        }
    }
    values.last().map(|(value, _)| *value)
}

fn weighted_average_f32(values: &[(f32, f64)]) -> Option<f32> {
    let normalized_weights = capped_normalized_weights(
        &values.iter().map(|(_, weight)| *weight).collect::<Vec<_>>(),
        REPRESENTATIVE_MAX_ASN_SHARE,
    );
    let mut weighted_sum = 0.0;
    let mut total_weight = 0.0;
    for ((value, _), weight) in values.iter().zip(normalized_weights.iter()) {
        if *weight <= 0.0 {
            continue;
        }
        weighted_sum += *value as f64 * *weight;
        total_weight += *weight;
    }
    (total_weight > 0.0).then(|| (weighted_sum / total_weight) as f32)
}

fn build_circuit_representative_metrics(
    qoo_profile: Option<&lqos_utils::qoo::QooProfile>,
) -> FxHashMap<i64, CircuitRepresentativeMetrics> {
    let Ok(now) = time_since_boot() else {
        return FxHashMap::default();
    };
    let recent_cutoff = Duration::from(now)
        .as_nanos()
        .saturating_sub((RETIRE_AFTER_SECONDS as u128) * 1_000_000_000)
        as u64;
    let shaped = SHAPED_DEVICES.load();
    let cache = SHAPED_DEVICE_HASH_CACHE.load();

    let mut buckets: FxHashMap<(i64, u32), RepresentativeAsnAggregate> = FxHashMap::default();
    let all_flows = ALL_FLOWS.lock();
    for (key, (local, analysis)) in all_flows.flow_data.iter() {
        if local.last_seen < recent_cutoff {
            continue;
        }

        let device = shaped_device_from_hashes_or_ip(
            &shaped,
            &cache,
            &key.local_ip,
            local.device_hash,
            local.circuit_hash,
        );
        let circuit_hash = local
            .circuit_hash
            .or_else(|| device.map(|device| device.circuit_hash));
        let Some(circuit_hash) = circuit_hash else {
            continue;
        };
        if crate::rtt_exclusions::is_excluded_hash(circuit_hash) {
            continue;
        }

        let bucket = buckets
            .entry((circuit_hash, analysis.asn_id.0))
            .or_default();
        bucket.total_bps.checked_add_direct(
            local.rate_estimate_bps.down as u64,
            local.rate_estimate_bps.up as u64,
        );

        let Some(tcp_info) = local.tcp_info.as_ref() else {
            continue;
        };
        bucket
            .tcp_packets
            .checked_add_direct(local.packets_sent.down, local.packets_sent.up);
        bucket.tcp_retransmits.checked_add_direct(
            local.tcp_retransmits.down as u64,
            local.tcp_retransmits.up as u64,
        );

        accumulate_representative_direction(
            bucket,
            &tcp_info.rtt,
            FlowbeeEffectiveDirection::Download,
            local.rate_estimate_bps.down,
            local.bytes_sent.down,
        );
        accumulate_representative_direction(
            bucket,
            &tcp_info.rtt,
            FlowbeeEffectiveDirection::Upload,
            local.rate_estimate_bps.up,
            local.bytes_sent.up,
        );
    }

    let mut by_circuit: FxHashMap<i64, Vec<RepresentativeAsnAggregate>> = FxHashMap::default();
    for ((circuit_hash, _asn), bucket) in buckets {
        by_circuit.entry(circuit_hash).or_default().push(bucket);
    }

    build_circuit_representative_metrics_from_buckets(by_circuit, qoo_profile)
}

fn build_circuit_representative_metrics_from_buckets(
    by_circuit: FxHashMap<i64, Vec<RepresentativeAsnAggregate>>,
    qoo_profile: Option<&lqos_utils::qoo::QooProfile>,
) -> FxHashMap<i64, CircuitRepresentativeMetrics> {
    let mut results = FxHashMap::default();
    for (circuit_hash, buckets) in by_circuit {
        let mut rtt_down = Vec::new();
        let mut rtt_up = Vec::new();
        let mut rtt_p90_down = Vec::new();
        let mut rtt_p90_up = Vec::new();
        let mut qoo_down = Vec::new();
        let mut qoo_up = Vec::new();

        for bucket in &buckets {
            if let Some(rtt) = bucket
                .rtt
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50)
                .map(|rtt| rtt.as_nanos())
                && let Some(weight) =
                    representative_weight(bucket.total_bps.down, bucket.rtt_visible_bps.down)
            {
                rtt_down.push((rtt, weight));
            }
            if let Some(rtt) = bucket
                .rtt
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 90)
                .map(|rtt| rtt.as_nanos())
                && let Some(weight) =
                    representative_weight(bucket.total_bps.down, bucket.rtt_visible_bps.down)
            {
                rtt_p90_down.push((rtt, weight));
            }
            if let Some(rtt) = bucket
                .rtt
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50)
                .map(|rtt| rtt.as_nanos())
                && let Some(weight) =
                    representative_weight(bucket.total_bps.up, bucket.rtt_visible_bps.up)
            {
                rtt_up.push((rtt, weight));
            }
            if let Some(rtt) = bucket
                .rtt
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 90)
                .map(|rtt| rtt.as_nanos())
                && let Some(weight) =
                    representative_weight(bucket.total_bps.up, bucket.rtt_visible_bps.up)
            {
                rtt_p90_up.push((rtt, weight));
            }

            let Some(profile) = qoo_profile else {
                continue;
            };
            let scores = compute_qoq_scores(
                profile,
                &bucket.rtt,
                tcp_retransmit_loss_proxy(bucket.tcp_retransmits.down, bucket.tcp_packets.down),
                tcp_retransmit_loss_proxy(bucket.tcp_retransmits.up, bucket.tcp_packets.up),
            );
            if let Some(score) = scores.download_total_f32()
                && let Some(weight) =
                    representative_weight(bucket.total_bps.down, bucket.rtt_visible_bps.down)
            {
                qoo_down.push((score, weight));
            }
            if let Some(score) = scores.upload_total_f32()
                && let Some(weight) =
                    representative_weight(bucket.total_bps.up, bucket.rtt_visible_bps.up)
            {
                qoo_up.push((score, weight));
            }
        }

        results.insert(
            circuit_hash,
            CircuitRepresentativeMetrics {
                rtt_current_p50_nanos: DownUpOrder {
                    down: weighted_median_u64(&mut rtt_down),
                    up: weighted_median_u64(&mut rtt_up),
                },
                rtt_current_p90_nanos: DownUpOrder {
                    down: weighted_median_u64(&mut rtt_p90_down),
                    up: weighted_median_u64(&mut rtt_p90_up),
                },
                qoo: DownUpOrder {
                    down: weighted_average_f32(&qoo_down),
                    up: weighted_average_f32(&qoo_up),
                },
            },
        );
    }

    results
}

struct ReducedHostCounters {
    bytes: DownUpOrder<u64>,
    actual_bytes: DownUpOrder<u64>,
    packets: DownUpOrder<u64>,
    tcp_packets: DownUpOrder<u64>,
    udp_packets: DownUpOrder<u64>,
    icmp_packets: DownUpOrder<u64>,
    last_seen: u64,
    tc_handle: TcHandle,
    circuit_hash: Option<i64>,
    device_hash: Option<i64>,
}

impl ReducedHostCounters {
    fn from_counters(counts: &[lqos_sys::HostCounter]) -> Self {
        let mut bytes = DownUpOrder::zeroed();
        let mut actual_bytes = DownUpOrder::zeroed();
        let mut packets = DownUpOrder::zeroed();
        let mut tcp_packets = DownUpOrder::zeroed();
        let mut udp_packets = DownUpOrder::zeroed();
        let mut icmp_packets = DownUpOrder::zeroed();
        let mut last_seen = 0u64;
        let mut meta_last_seen = 0u64;
        let mut meta_tc_handle = 0u32;
        let mut meta_circuit_id = 0u64;
        let mut meta_device_id = 0u64;

        for c in counts {
            bytes.checked_add_direct(c.download_bytes, c.upload_bytes);
            actual_bytes.checked_add_direct(c.actual_download_bytes, c.actual_upload_bytes);
            packets.checked_add_direct(c.download_packets, c.upload_packets);
            tcp_packets.checked_add_direct(c.tcp_download_packets, c.tcp_upload_packets);
            udp_packets.checked_add_direct(c.udp_download_packets, c.udp_upload_packets);
            icmp_packets.checked_add_direct(c.icmp_download_packets, c.icmp_upload_packets);
            last_seen = u64::max(last_seen, c.last_seen);
            if c.last_seen > meta_last_seen {
                meta_last_seen = c.last_seen;
                meta_tc_handle = c.tc_handle;
                meta_circuit_id = c.circuit_id;
                meta_device_id = c.device_id;
            }
        }

        Self {
            bytes,
            actual_bytes,
            packets,
            tcp_packets,
            udp_packets,
            icmp_packets,
            last_seen,
            tc_handle: TcHandle::from_u32(meta_tc_handle),
            circuit_hash: (meta_circuit_id != 0).then_some(meta_circuit_id as i64),
            device_hash: (meta_device_id != 0).then_some(meta_device_id as i64),
        }
    }
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
            actual_bytes_per_second: AtomicDownUp::zeroed(),
            packets_per_second: AtomicDownUp::zeroed(),
            tcp_packets_per_second: AtomicDownUp::zeroed(),
            udp_packets_per_second: AtomicDownUp::zeroed(),
            icmp_packets_per_second: AtomicDownUp::zeroed(),
            shaped_bytes_per_second: AtomicDownUp::zeroed(),
            shaped_actual_bytes_per_second: AtomicDownUp::zeroed(),
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
        let representative_snapshot = build_circuit_representative_metrics(qoo_profile.as_deref());
        CIRCUIT_REPRESENTATIVE_METRICS.store(Arc::new(representative_snapshot.clone()));
        {
            let raw_data = self.raw_data.lock();
            for entry in raw_data.values() {
                let circuit_hash = if let Some(circuit_hash) = entry.circuit_hash {
                    circuit_hash
                } else {
                    continue;
                };

                let download_delta = entry
                    .actual_bytes
                    .down
                    .saturating_sub(entry.prev_actual_bytes.down);
                let upload_delta = entry
                    .actual_bytes
                    .up
                    .saturating_sub(entry.prev_actual_bytes.up);
                total_download_bytes = total_download_bytes.saturating_add(download_delta);
                total_upload_bytes = total_upload_bytes.saturating_add(upload_delta);
                total_tcp_packets.down = total_tcp_packets
                    .down
                    .saturating_add(entry.tcp_retransmit_packets.down);
                total_tcp_packets.up = total_tcp_packets
                    .up
                    .saturating_add(entry.tcp_retransmit_packets.up);
                total_retransmits.down = total_retransmits
                    .down
                    .saturating_add(entry.tcp_retransmits.down);
                total_retransmits.up = total_retransmits
                    .up
                    .saturating_add(entry.tcp_retransmits.up);

                let agg = aggregates.entry(circuit_hash).or_default();
                agg.download_bytes = agg.download_bytes.saturating_add(download_delta);
                agg.upload_bytes = agg.upload_bytes.saturating_add(upload_delta);
                agg.tcp_packets.down = agg
                    .tcp_packets
                    .down
                    .saturating_add(entry.tcp_retransmit_packets.down);
                agg.tcp_packets.up = agg
                    .tcp_packets
                    .up
                    .saturating_add(entry.tcp_retransmit_packets.up);
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
        for (circuit_hash, aggregate) in aggregates {
            let (max_down_mbps, max_up_mbps) = capacity_lookup
                .get(&circuit_hash)
                .copied()
                .unwrap_or((0.0, 0.0));

            let download_util =
                utilization_percent(aggregate.download_bytes, max_down_mbps).unwrap_or(0.0);
            let upload_util =
                utilization_percent(aggregate.upload_bytes, max_up_mbps).unwrap_or(0.0);
            let representative = representative_snapshot.get(&circuit_hash);
            let rtt_p50_down = representative
                .and_then(|metrics| metrics.rtt_current_p50_nanos.down)
                .map(|rtt| RttData::from_nanos(rtt).as_millis() as f32);
            let rtt_p50_up = representative
                .and_then(|metrics| metrics.rtt_current_p50_nanos.up)
                .map(|rtt| RttData::from_nanos(rtt).as_millis() as f32);
            let rtt_p90_down = representative
                .and_then(|metrics| metrics.rtt_current_p90_nanos.down)
                .map(|rtt| RttData::from_nanos(rtt).as_millis() as f32);
            let rtt_p90_up = representative
                .and_then(|metrics| metrics.rtt_current_p90_nanos.up)
                .map(|rtt| RttData::from_nanos(rtt).as_millis() as f32);
            let retransmit_down =
                retransmit_percent(aggregate.tcp_retransmits.down, aggregate.tcp_packets.down);
            let retransmit_up =
                retransmit_percent(aggregate.tcp_retransmits.up, aggregate.tcp_packets.up);

            let heatmap = heatmaps.entry(circuit_hash).or_default();
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

            let qoq_heatmap = qoq_heatmaps.entry(circuit_hash).or_default();
            qoq_heatmap.add_sample(
                representative.and_then(|metrics| metrics.qoo.down),
                representative.and_then(|metrics| metrics.qoo.up),
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

        self.global_qoq_heatmap
            .lock()
            .add_sample(scores.download_total_f32(), scores.upload_total_f32());
    }

    pub(crate) fn copy_previous_and_reset_rtt(&self) {
        // Copy previous byte/packet numbers and reset RTT data
        let self_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);
        let mut raw_data = self.raw_data.lock();
        raw_data.iter_mut().for_each(|(_k, v)| {
            if v.first_cycle < self_cycle {
                v.bytes_per_second = v.bytes.checked_sub_or_zero(v.prev_bytes);
                v.actual_bytes_per_second = v.actual_bytes.checked_sub_or_zero(v.prev_actual_bytes);
                v.packets_per_second = v.packets.checked_sub_or_zero(v.prev_packets);
            }
            v.prev_bytes = v.bytes;
            v.prev_actual_bytes = v.actual_bytes;
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

    fn shaped_device_for_ip_or_hashes<'a>(
        shaped: &'a lqos_config::ConfigShapedDevices,
        cache: &crate::shaped_devices_tracker::ShapedDeviceHashCache,
        ip: &XdpIpAddress,
        device_hash: Option<i64>,
        circuit_hash: Option<i64>,
    ) -> Option<&'a lqos_config::ShapedDevice> {
        if let Some(device_hash) = device_hash
            && let Some(idx) = cache.index_by_device_hash(shaped, device_hash)
        {
            return shaped.devices.get(idx);
        }
        if let Some(circuit_hash) = circuit_hash
            && let Some(idx) = cache.index_by_circuit_hash(shaped, circuit_hash)
        {
            return shaped.devices.get(idx);
        }
        shaped.get_device_from_ip(ip)
    }

    fn lookup_network_parents_from_ip_or_hashes(
        shaped: &lqos_config::ConfigShapedDevices,
        cache: &crate::shaped_devices_tracker::ShapedDeviceHashCache,
        ip: &XdpIpAddress,
        device_hash: Option<i64>,
        circuit_hash: Option<i64>,
        lock: &NetworkJson,
    ) -> Option<Vec<usize>> {
        Self::shaped_device_for_ip_or_hashes(shaped, cache, ip, device_hash, circuit_hash)
            .and_then(|device| lock.get_parents_for_circuit_id(&device.parent_node))
    }

    pub(crate) fn refresh_circuit_ids(&self, lock: &NetworkJson) {
        let shaped = SHAPED_DEVICES.load();
        let cache = SHAPED_DEVICE_HASH_CACHE.load();
        let mut raw_data = self.raw_data.lock();
        raw_data.iter_mut().for_each(|(ip, data)| {
            let shaped_device = Self::shaped_device_for_ip_or_hashes(
                &shaped,
                &cache,
                ip,
                data.device_hash,
                data.circuit_hash,
            );
            if data.device_hash.is_none()
                && let Some(device) = shaped_device
            {
                data.device_hash = Some(device.device_hash);
            }
            if data.circuit_hash.is_none()
                && let Some(device) = shaped_device
            {
                data.circuit_hash = Some(device.circuit_hash);
            }
            data.circuit_id = shaped_device.map(|d| d.circuit_id.clone());
            data.network_json_parents = shaped_device
                .and_then(|device| lock.get_parents_for_circuit_id(&device.parent_node));
        });
    }

    pub(crate) fn apply_new_throughput_counters(
        &self,
        net_json_calc: &mut NetworkJson,
        bakery_sender: crossbeam_channel::Sender<BakeryCommands>,
    ) {
        let mut changed_circuits = HashSet::new();

        let self_cycle = self.cycle.load(std::sync::atomic::Ordering::Relaxed);
        let shaped = SHAPED_DEVICES.load();
        let cache = SHAPED_DEVICE_HASH_CACHE.load();
        let mut raw_data = self.raw_data.lock();
        throughput_for_each(&mut |xdp_ip, counts| {
            let reduced = ReducedHostCounters::from_counters(counts);
            if let Some(entry) = raw_data.get_mut(xdp_ip) {
                // Zero the counter, we have to do a per-CPU sum
                entry.bytes = reduced.bytes;
                entry.actual_bytes = reduced.actual_bytes;
                entry.packets = reduced.packets;
                entry.tcp_packets = reduced.tcp_packets;
                entry.udp_packets = reduced.udp_packets;
                entry.icmp_packets = reduced.icmp_packets;
                entry.last_seen = reduced.last_seen;

                entry.tc_handle = reduced.tc_handle;
                let shaped_device = Self::shaped_device_for_ip_or_hashes(
                    &shaped,
                    &cache,
                    xdp_ip,
                    reduced.device_hash,
                    reduced.circuit_hash,
                );
                let resolved_circuit_hash = reduced
                    .circuit_hash
                    .or_else(|| shaped_device.map(|device| device.circuit_hash));
                let resolved_device_hash = reduced
                    .device_hash
                    .or_else(|| shaped_device.map(|device| device.device_hash));
                let hashes_changed = entry.circuit_hash != resolved_circuit_hash
                    || entry.device_hash != resolved_device_hash;
                entry.circuit_hash = resolved_circuit_hash;
                entry.device_hash = resolved_device_hash;
                if hashes_changed {
                    entry.circuit_id = shaped_device.map(|d| d.circuit_id.clone());
                    entry.network_json_parents = shaped_device.and_then(|device| {
                        net_json_calc.get_parents_for_circuit_id(&device.parent_node)
                    });
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
                                entry
                                    .actual_bytes
                                    .down
                                    .saturating_sub(entry.prev_actual_bytes.down),
                                entry
                                    .actual_bytes
                                    .up
                                    .saturating_sub(entry.prev_actual_bytes.up),
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
                let shaped_device = Self::shaped_device_for_ip_or_hashes(
                    &shaped,
                    &cache,
                    xdp_ip,
                    reduced.device_hash,
                    reduced.circuit_hash,
                );
                let circuit_hash = reduced
                    .circuit_hash
                    .or_else(|| shaped_device.map(|device| device.circuit_hash));
                let device_hash = reduced
                    .device_hash
                    .or_else(|| shaped_device.map(|device| device.device_hash));
                let circuit_id = shaped_device.map(|d| d.circuit_id.clone());
                // Call the Bakery Queue Creation for new circuits
                if let Some(circuit_hash) = circuit_hash
                    && let Ok(config) = lqos_config::load_config()
                    && config.queues.lazy_queues.is_some()
                {
                    let mut add = true;

                    if config.queues.lazy_threshold_bytes.is_some() {
                        let threshold = config.queues.lazy_threshold_bytes.unwrap_or(0);
                        if reduced.bytes.down.saturating_add(reduced.bytes.up) < threshold {
                            add = false;
                        }
                    }

                    if add {
                        changed_circuits.insert(circuit_hash);
                    }
                }
                let entry = ThroughputEntry {
                    circuit_id,
                    circuit_hash,
                    device_hash,
                    network_json_parents: Self::lookup_network_parents_from_ip_or_hashes(
                        &shaped,
                        &cache,
                        xdp_ip,
                        device_hash,
                        circuit_hash,
                        net_json_calc,
                    ),
                    first_cycle: self_cycle,
                    most_recent_cycle: 0,
                    bytes: reduced.bytes,
                    actual_bytes: reduced.actual_bytes,
                    packets: reduced.packets,
                    prev_bytes: DownUpOrder::zeroed(),
                    prev_actual_bytes: DownUpOrder::zeroed(),
                    prev_packets: DownUpOrder::zeroed(),
                    bytes_per_second: DownUpOrder::zeroed(),
                    actual_bytes_per_second: DownUpOrder::zeroed(),
                    packets_per_second: DownUpOrder::zeroed(),
                    tcp_packets: reduced.tcp_packets,
                    udp_packets: reduced.udp_packets,
                    icmp_packets: reduced.icmp_packets,
                    prev_tcp_packets: DownUpOrder::zeroed(),
                    prev_udp_packets: DownUpOrder::zeroed(),
                    prev_icmp_packets: DownUpOrder::zeroed(),
                    tc_handle: reduced.tc_handle,
                    rtt_buffer: RttBuffer::default(),
                    recent_rtt_data: [RttData::from_nanos(0); 60],
                    last_fresh_rtt_data_cycle: 0,
                    last_seen: reduced.last_seen,
                    tcp_retransmits: DownUpOrder::zeroed(),
                    tcp_retransmit_packets: DownUpOrder::zeroed(),
                    qoq: QoqScores::default(),
                };
                raw_data.insert(*xdp_ip, entry);
            }
        });

        if !changed_circuits.is_empty()
            && let Err(e) = bakery_sender.send(BakeryCommands::OnCircuitActivity {
                circuit_ids: changed_circuits,
            })
        {
            warn!("Failed to send BakeryCommands::OnCircuitActivity: {:?}", e);
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

    pub(crate) fn apply_flow_data(&self, ctx: FlowApplyContext<'_>) {
        let FlowApplyContext {
            timeout_seconds,
            sender,
            net_json_calc,
            rtt_circuit_tracker,
            rtt_by_circuit,
            tcp_retries,
            tcp_retry_packets,
            expired_keys,
        } = ctx;
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
            let agg = asn_aggregates.entry(asn).or_default();
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
                    expired_keys.push(*key);
                } else {
                    // We have a valid flow, so it needs to be tracked
                    if let Some(this_flow) = all_flows_lock.flow_data.get_mut(key) {
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
                            this_flow.0.record_tcp_retry_time(
                                FlowbeeEffectiveDirection::Download,
                                data.last_seen,
                            );
                        }
                        if data.tcp_retransmits.up != this_flow.0.tcp_retransmits.up {
                            this_flow.0.record_tcp_retry_time(
                                FlowbeeEffectiveDirection::Upload,
                                data.last_seen,
                            );
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
                        if key.ip_protocol == 6
                            && let (Some(profile), Some(tcp_info)) =
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
                        if enable_asn_heatmaps {
                            let excluded = raw_data
                                .get(&key.local_ip)
                                .and_then(|t| t.circuit_hash)
                                .is_some_and(crate::rtt_exclusions::is_excluded_hash);
                            let flow_rtt = if excluded {
                                None
                            } else {
                                combine_rtt_ms(this_flow.0.get_rtt_array())
                            };
                            add_asn_sample(
                                this_flow.1.asn_id.0,
                                delta_bytes,
                                delta_packets,
                                delta_retrans,
                                flow_rtt,
                            );
                        }
                        if key.ip_protocol == 6
                            && data.end_status == 0
                            && raw_data.contains_key(&key.local_ip)
                        {
                            tcp_retries
                                .entry(key.local_ip)
                                .or_insert_with(DownUpOrder::zeroed)
                                .checked_add(delta_retrans);
                            tcp_retry_packets
                                .entry(key.local_ip)
                                .or_insert_with(DownUpOrder::zeroed)
                                .checked_add(delta_packets);
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
                            let flow_analysis = FlowAnalysis::new(key);
                            let mut flow_summary = FlowbeeLocalData::from_flow(data, key);
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
                            if key.ip_protocol == 6
                                && let (Some(profile), Some(tcp_info)) =
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
                            if enable_asn_heatmaps {
                                let excluded = raw_data
                                    .get(&key.local_ip)
                                    .and_then(|t| t.circuit_hash)
                                    .is_some_and(crate::rtt_exclusions::is_excluded_hash);
                                let flow_rtt = if excluded {
                                    None
                                } else {
                                    rtt_for_circuit.and_then(combine_rtt_ms)
                                };
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
                                .insert(*key, (flow_summary, flow_analysis));
                        }
                    }

                    if data.end_status != 0 {
                        // The flow has ended. We need to remove it from the map.
                        expired_keys.push(*key);
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

                if let Some(rtt_median) = rtt_median
                    && let Some(tracker) = raw_data.get_mut(local_ip)
                {
                    // Shift left
                    for i in 1..60 {
                        tracker.recent_rtt_data[i] = tracker.recent_rtt_data[i - 1];
                    }
                    tracker.recent_rtt_data[0] = rtt_median;
                    tracker.last_fresh_rtt_data_cycle = self_cycle;
                    tracker.rtt_buffer = rtt_buffer;
                    let excluded = tracker
                        .circuit_hash
                        .is_some_and(crate::rtt_exclusions::is_excluded_hash);
                    if !excluded {
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
                circuit.tcp_retransmit_packets = DownUpOrder::zeroed();
            }
            // Apply the new ones
            for (local_ip, retries) in tcp_retries {
                if let Some(tracker) = raw_data.get_mut(local_ip) {
                    tracker.tcp_retransmit_packets = tcp_retry_packets
                        .get(local_ip)
                        .copied()
                        .unwrap_or_else(DownUpOrder::zeroed);
                    tracker.tcp_retransmits = *retries;

                    // Send it upstream
                    if let Some(parents) = &tracker.network_json_parents {
                        net_json_calc.add_retransmit_cycle(
                            parents,
                            tracker.tcp_retransmits,
                            tracker.tcp_retransmit_packets,
                        );
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
                    if tracker
                        .circuit_hash
                        .is_some_and(crate::rtt_exclusions::is_excluded_hash)
                    {
                        tracker.qoq = QoqScores::default();
                        continue;
                    }
                    let tcp_packets_delta = tracker.tcp_retransmit_packets;
                    let loss_download = tcp_retransmit_loss_proxy(
                        tracker.tcp_retransmits.down,
                        tcp_packets_delta.down,
                    );
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
                    if let Some(d) = all_flows_lock.flow_data.remove(key) {
                        let _ = sender.send((*key, (d.0.clone(), d.1)));
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
        self.actual_bytes_per_second.set_to_zero();
        self.packets_per_second.set_to_zero();
        self.tcp_packets_per_second.set_to_zero();
        self.udp_packets_per_second.set_to_zero();
        self.icmp_packets_per_second.set_to_zero();
        self.shaped_bytes_per_second.set_to_zero();
        self.shaped_actual_bytes_per_second.set_to_zero();
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
                    v.actual_bytes.down.saturating_sub(v.prev_actual_bytes.down),
                    v.actual_bytes.up.saturating_sub(v.prev_actual_bytes.up),
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
                    actual_bytes_down,
                    actual_bytes_up,
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
                    self.actual_bytes_per_second
                        .checked_add_tuple((actual_bytes_down, actual_bytes_up));
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
                        self.shaped_actual_bytes_per_second
                            .checked_add_tuple((actual_bytes_down, actual_bytes_up));
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
                    keys_to_expire.push(*k);
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

    #[allow(dead_code)]
    pub(crate) fn actual_bits_per_second(&self) -> DownUpOrder<u64> {
        self.actual_bytes_per_second
            .as_down_up()
            .to_bits_from_bytes()
    }

    #[allow(dead_code)]
    pub(crate) fn shaped_bits_per_second(&self) -> DownUpOrder<u64> {
        self.shaped_bytes_per_second
            .as_down_up()
            .to_bits_from_bytes()
    }

    #[allow(dead_code)]
    pub(crate) fn shaped_actual_bits_per_second(&self) -> DownUpOrder<u64> {
        self.shaped_actual_bytes_per_second
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
    let value = (retransmits as f32 / packets as f32) * 100.0;
    if value > 50.0 { None } else { Some(value) }
}

fn median(values: &mut [f32]) -> Option<f32> {
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

pub(crate) fn tcp_retransmit_loss_proxy(retransmits: u64, packets: u64) -> Option<LossMeasurement> {
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

#[cfg(test)]
mod tests {
    use super::{
        REPRESENTATIVE_MAX_ASN_SHARE, REPRESENTATIVE_MIN_FLOW_BYTES, RepresentativeAsnAggregate,
        ThroughputTracker, accumulate_representative_direction,
        build_circuit_representative_metrics_from_buckets, capped_normalized_weights,
        representative_weight,
    };
    use crate::shaped_devices_tracker::ShapedDeviceHashCache;
    use crate::throughput_tracker::flow_data::{FlowbeeEffectiveDirection, RttData};
    use lqos_config::{ConfigShapedDevices, ShapedDevice};
    use lqos_utils::rtt::RttBucket;
    use lqos_utils::rtt::RttBuffer;
    use lqos_utils::{XdpIpAddress, hash_to_i64};
    use std::net::Ipv4Addr;

    #[test]
    fn shaped_device_lookup_falls_back_to_ip_when_hashes_missing() {
        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(vec![ShapedDevice {
            circuit_id: "circuit-1".to_string(),
            device_id: "device-1".to_string(),
            parent_node: "Parent-A".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 168, 1, 10), 32)],
            ..Default::default()
        }]);
        let cache = ShapedDeviceHashCache::default();
        let ip = XdpIpAddress::from_ip("192.168.1.10".parse().expect("test IP should parse"));

        let matched =
            ThroughputTracker::shaped_device_for_ip_or_hashes(&shaped, &cache, &ip, None, None)
                .expect("lookup should resolve by IP");

        assert_eq!(matched.circuit_hash, hash_to_i64("circuit-1"));
        assert_eq!(matched.device_hash, hash_to_i64("device-1"));
    }

    #[test]
    fn representative_weight_penalizes_low_visibility() {
        let high_visibility =
            representative_weight(100_000_000, 100_000_000).expect("weight should exist");
        let low_visibility =
            representative_weight(100_000_000, 10_000_000).expect("weight should exist");

        assert!(high_visibility > low_visibility);
    }

    #[test]
    fn representative_weight_growth_is_flatter_than_square_root() {
        let medium = representative_weight(10_000_000, 10_000_000).expect("weight should exist");
        let large =
            representative_weight(1_000_000_000, 1_000_000_000).expect("weight should exist");

        assert!(large > medium);
        assert!(large / medium < 2.0);
    }

    #[test]
    fn representative_direction_ignores_tiny_rtt_bearing_flows() {
        let mut bucket = RepresentativeAsnAggregate::default();
        let mut rtt = RttBuffer::default();
        rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );

        accumulate_representative_direction(
            &mut bucket,
            &rtt,
            FlowbeeEffectiveDirection::Download,
            750_000,
            REPRESENTATIVE_MIN_FLOW_BYTES - 1,
        );

        assert_eq!(bucket.rtt_visible_bps.down, 0);
        assert!(
            bucket
                .rtt
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50)
                .is_none()
        );
    }

    #[test]
    fn representative_direction_counts_flows_over_minimum_floor() {
        let mut bucket = RepresentativeAsnAggregate::default();
        let mut rtt = RttBuffer::default();
        rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );

        accumulate_representative_direction(
            &mut bucket,
            &rtt,
            FlowbeeEffectiveDirection::Download,
            750_000,
            REPRESENTATIVE_MIN_FLOW_BYTES,
        );

        assert_eq!(bucket.rtt_visible_bps.down, 750_000);
        assert!(
            bucket
                .rtt
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50)
                .is_some()
        );
    }

    #[test]
    fn capped_normalized_weights_limit_single_asn_share() {
        let normalized = capped_normalized_weights(
            &[100.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0],
            REPRESENTATIVE_MAX_ASN_SHARE,
        );

        assert_eq!(normalized.len(), 7);
        assert!(
            normalized
                .iter()
                .all(|weight| *weight <= REPRESENTATIVE_MAX_ASN_SHARE + 0.0000001)
        );
        let total: f64 = normalized.iter().sum();
        assert!((total - 1.0).abs() < 0.000001);
    }

    #[test]
    fn capped_normalized_weights_fall_back_when_cap_is_infeasible() {
        let normalized =
            capped_normalized_weights(&[100.0, 10.0, 10.0, 10.0], REPRESENTATIVE_MAX_ASN_SHARE);

        assert_eq!(normalized.len(), 4);
        let total: f64 = normalized.iter().sum();
        assert!((total - 1.0).abs() < 0.000001);
        assert!(normalized[0] > REPRESENTATIVE_MAX_ASN_SHARE);
    }

    #[test]
    fn representative_metrics_cap_single_dominant_asn() {
        let mut trusted = RepresentativeAsnAggregate::default();
        trusted.total_bps.down = 10_000_000;
        trusted.rtt_visible_bps.down = 10_000_000;
        trusted.rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        trusted.rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );

        let mut trusted2 = RepresentativeAsnAggregate::default();
        trusted2.total_bps.down = 10_000_000;
        trusted2.rtt_visible_bps.down = 10_000_000;
        trusted2.rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        trusted2.rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );

        let mut trusted3 = RepresentativeAsnAggregate::default();
        trusted3.total_bps.down = 10_000_000;
        trusted3.rtt_visible_bps.down = 10_000_000;
        trusted3.rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        trusted3.rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );

        let mut noisy = RepresentativeAsnAggregate::default();
        noisy.total_bps.down = 500_000_000;
        noisy.rtt_visible_bps.down = 500_000_000;
        noisy.rtt.push(
            RttData::from_nanos(999_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        noisy.rtt.push(
            RttData::from_nanos(999_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );

        let mut by_circuit = fxhash::FxHashMap::default();
        by_circuit.insert(1_i64, vec![trusted, trusted2, trusted3, noisy]);
        let metrics = build_circuit_representative_metrics_from_buckets(by_circuit, None);
        let circuit = metrics.get(&1_i64).expect("circuit should exist");

        assert_eq!(circuit.rtt_current_p50_nanos.down, Some(35_000_000));
    }
}
