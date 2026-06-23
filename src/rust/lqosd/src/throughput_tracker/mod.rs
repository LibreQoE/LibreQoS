pub mod flow_data;
mod stats_submission;
mod throughput_entry;
mod tracking_data;

use self::flow_data::{
    ActiveFlowSnapshot, FlowAnalysis, FlowbeeLocalData, active_flow_snapshot, get_asn_name_by_id,
    live_active_flow_count, refresh_active_flow_snapshot, snapshot_asn_heatmaps,
};
use self::throughput_entry::ThroughputEntry;
use crate::system_stats::SystemStats;
use crate::throughput_tracker::flow_data::FlowbeeEffectiveDirection;
use crate::{
    stats::TIME_TO_POLL_HOSTS,
    throughput_tracker::tracking_data::{FlowApplyContext, ThroughputTracker},
};
use arc_swap::ArcSwap;
pub(crate) use flow_data::RttBuffer;
use fxhash::{FxHashMap, FxHashSet};
use lqos_bakery::{BakeryCommands, full_reload_in_progress};
use lqos_bus::{
    AsnHeatmapData, BusResponse, CircuitHeatmapData, ExecutiveSummaryHeader, IpStats,
    SiteHeatmapData, TcHandle, TopFlowType, XdpPpingResult,
};
use lqos_queue_tracker::{ALL_QUEUE_SUMMARY, queue_stats_stale};
use lqos_sys::flowbee_data::FlowbeeKey;
#[cfg(test)]
use lqos_utils::qoo::QoqScores;
use lqos_utils::rtt::RttBucket;
#[cfg(test)]
use lqos_utils::rtt::RttData;
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample, down_up_retransmit_sample};
use lqos_utils::{XdpIpAddress, hash_to_i64, unix_time::time_since_boot};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::net::IpAddr;
use std::sync::Arc;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use tokio::time::{Duration, Instant};
use tracing::{debug, info, warn};

const RETIRE_AFTER_SECONDS: u64 = 30;
const RELOAD_THROUGHPUT_POLL_INTERVAL_SECONDS: u64 = 5;
type FinishedFlowExport = (FlowbeeKey, (FlowbeeLocalData, FlowAnalysis));

pub static THROUGHPUT_TRACKER: Lazy<ThroughputTracker> = Lazy::new(ThroughputTracker::new);
pub(crate) static CIRCUIT_RTT_BUFFERS: Lazy<ArcSwap<FxHashMap<i64, RttBuffer>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(FxHashMap::default())));

/// Returns the current per-circuit RTT p50 values from the shared circuit RTT buffers.
pub(crate) fn circuit_current_rtt_p50_nanos(circuit_hash: i64) -> DownUpOrder<Option<u64>> {
    let snapshot = CIRCUIT_RTT_BUFFERS.load();
    let rtt = snapshot.get(&circuit_hash);

    DownUpOrder {
        down: rtt
            .and_then(|rtt| {
                rtt.percentile(RttBucket::Current, FlowbeeEffectiveDirection::Download, 50)
            })
            .map(|rtt| rtt.as_nanos()),
        up: rtt
            .and_then(|rtt| {
                rtt.percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50)
            })
            .map(|rtt| rtt.as_nanos()),
    }
}

/// Returns the current per-circuit QoO values from the shared circuit QoO heatmaps.
pub(crate) fn circuit_current_qoo(circuit_hash: i64) -> DownUpOrder<Option<f32>> {
    let qoq_heatmaps = THROUGHPUT_TRACKER.circuit_qoq_heatmaps.lock();
    let Some(heatmap) = qoq_heatmaps.get(&circuit_hash) else {
        return DownUpOrder::default();
    };
    let blocks = heatmap.blocks();
    DownUpOrder {
        down: blocks.download_total.last().copied().flatten(),
        up: blocks.upload_total.last().copied().flatten(),
    }
}

fn finish_expired_flows(
    finished_flow_exports: &mut Vec<FinishedFlowExport>,
    expired_flows: &mut [FlowbeeKey],
    netflow_sender: &crossbeam_channel::Sender<FinishedFlowExport>,
    end_flows: impl FnOnce(&mut [FlowbeeKey]) -> anyhow::Result<()>,
) {
    for export in finished_flow_exports.drain(..) {
        if let Err(e) = netflow_sender.send(export) {
            warn!("Failed to send finished flow export: {:?}", e);
        }
    }

    if !expired_flows.is_empty()
        && let Err(e) = end_flows(expired_flows)
    {
        warn!("Failed to end flows: {:?}", e);
    }
}

fn dedup_flow_keys(keys: &mut Vec<FlowbeeKey>) {
    let mut seen = FxHashSet::default();
    keys.retain(|key| seen.insert(*key));
}

/// Resolves the shaped-device row for an active flow from its hashes, then its local IP.
pub(crate) fn resolve_flow_device<'a>(
    catalog: &'a lqos_network_devices::NetworkDevicesCatalog,
    ip: &XdpIpAddress,
    device_hash: Option<i64>,
    circuit_hash: Option<i64>,
) -> Option<&'a lqos_config::ShapedDevice> {
    catalog
        .device_by_hashes(device_hash, circuit_hash)
        .or_else(|| catalog.device_longest_match_for_ip(ip).map(|(_, dev)| dev))
}

/// Resolves circuit ID/name metadata for an active-flow or throughput row.
pub(crate) fn flow_circuit_metadata_from_device(
    device: Option<&lqos_config::ShapedDevice>,
    circuit_id_hint: Option<&str>,
) -> (String, String) {
    let mut circuit_id = circuit_id_hint.unwrap_or_default().to_string();
    let mut circuit_name = String::new();

    if let Some(device) = device {
        if circuit_id.is_empty() {
            circuit_id = device.circuit_id.clone();
        }
        circuit_name = device.circuit_name.clone();
    }

    (circuit_id, circuit_name)
}

fn resolve_circuit_metadata_for_entry(
    catalog: &lqos_network_devices::NetworkDevicesCatalog,
    ip: &XdpIpAddress,
    entry: &ThroughputEntry,
) -> (String, String) {
    flow_circuit_metadata_from_device(
        resolve_flow_device(catalog, ip, entry.device_hash, entry.circuit_hash),
        entry.circuit_id.as_deref(),
    )
}

pub(crate) fn resolve_circuit_metadata_for_ip(ip: &XdpIpAddress) -> (String, String) {
    let catalog = lqos_network_devices::network_devices_catalog();
    let throughput = THROUGHPUT_TRACKER.raw_data.lock();
    let Some(entry) = throughput.get(ip) else {
        return (String::new(), String::new());
    };
    resolve_circuit_metadata_for_entry(&catalog, ip, entry)
}

#[cfg(test)]
pub(crate) struct RawThroughputTestEntry {
    pub(crate) ip: XdpIpAddress,
    pub(crate) circuit_hash: Option<i64>,
    pub(crate) device_hash: Option<i64>,
    pub(crate) most_recent_cycle: u64,
    pub(crate) bytes_per_second: DownUpOrder<u64>,
    pub(crate) tcp_packets: DownUpOrder<u64>,
    pub(crate) tcp_retransmits: DownUpOrder<u64>,
}

#[cfg(test)]
pub(crate) struct RawThroughputTestGuard {
    old_cycle: u64,
    old_raw_data: Option<std::collections::HashMap<XdpIpAddress, ThroughputEntry>>,
}

#[cfg(test)]
impl Drop for RawThroughputTestGuard {
    fn drop(&mut self) {
        THROUGHPUT_TRACKER
            .cycle
            .store(self.old_cycle, std::sync::atomic::Ordering::Relaxed);
        if let Some(old_raw_data) = self.old_raw_data.take() {
            *THROUGHPUT_TRACKER.raw_data.lock() = old_raw_data;
        }
    }
}

#[cfg(test)]
pub(crate) fn replace_raw_throughput_for_test(
    cycle: u64,
    entries: Vec<RawThroughputTestEntry>,
) -> RawThroughputTestGuard {
    let old_cycle = THROUGHPUT_TRACKER
        .cycle
        .swap(cycle, std::sync::atomic::Ordering::Relaxed);
    let mut raw_data = THROUGHPUT_TRACKER.raw_data.lock();
    let old_raw_data = std::mem::take(&mut *raw_data);
    for entry in entries {
        raw_data.insert(
            entry.ip,
            ThroughputEntry {
                circuit_id: None,
                circuit_hash: entry.circuit_hash,
                device_hash: entry.device_hash,
                network_json_parents: None,
                first_cycle: 0,
                most_recent_cycle: entry.most_recent_cycle,
                bytes: DownUpOrder::zeroed(),
                actual_bytes: DownUpOrder::zeroed(),
                packets: DownUpOrder::zeroed(),
                tcp_packets: entry.tcp_packets,
                udp_packets: DownUpOrder::zeroed(),
                icmp_packets: DownUpOrder::zeroed(),
                prev_bytes: DownUpOrder::zeroed(),
                prev_actual_bytes: DownUpOrder::zeroed(),
                prev_packets: DownUpOrder::zeroed(),
                prev_tcp_packets: DownUpOrder::zeroed(),
                prev_udp_packets: DownUpOrder::zeroed(),
                prev_icmp_packets: DownUpOrder::zeroed(),
                bytes_per_second: entry.bytes_per_second,
                actual_bytes_per_second: DownUpOrder::zeroed(),
                packets_per_second: DownUpOrder::zeroed(),
                tc_handle: TcHandle::from_u32(0),
                rtt_buffer: RttBuffer::default(),
                recent_rtt_data: [RttData::from_nanos(0); 60],
                last_fresh_rtt_data_cycle: 0,
                last_seen: 0,
                tcp_retransmits: entry.tcp_retransmits,
                tcp_retransmit_packets: DownUpOrder::zeroed(),
                qoq: QoqScores::default(),
            },
        );
    }
    RawThroughputTestGuard {
        old_cycle,
        old_raw_data: Some(old_raw_data),
    }
}

/// Create the throughput monitor thread, and begin polling for
/// throughput data every second.
///
/// ## Arguments
///
/// * `long_term_stats_tx` - an optional MPSC sender to notify the
///   collection thread that there is fresh data.
pub fn spawn_throughput_monitor(
    netflow_sender: crossbeam_channel::Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
    system_usage_actor: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
    bakery_sender: crossbeam_channel::Sender<lqos_bakery::BakeryCommands>,
) -> anyhow::Result<()> {
    debug!("Starting the bandwidth monitor thread.");
    std::thread::Builder::new()
        .name("Throughput Monitor".to_string())
        .spawn(|| throughput_task(netflow_sender, system_usage_actor, bakery_sender))?;

    Ok(())
}

/// Used for tracking the "tick" time, with a view to
/// finding where some code is stalling.
#[derive(Debug)]
struct ThroughputTaskTimeMetrics {
    start: Instant,
    update_cycle: f64,
    zero_throughput_and_rtt: f64,
    copy_previous_and_reset_rtt: f64,
    apply_new_throughput_counters: f64,
    apply_flow_data: f64,
    apply_queue_stats: f64,
    update_totals: f64,
    next_cycle: f64,
    finish_update_cycle: f64,
    lts_submit: f64,
}

impl ThroughputTaskTimeMetrics {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            update_cycle: 0.0,
            zero_throughput_and_rtt: 0.0,
            copy_previous_and_reset_rtt: 0.0,
            apply_new_throughput_counters: 0.0,
            apply_flow_data: 0.0,
            apply_queue_stats: 0.0,
            update_totals: 0.0,
            next_cycle: 0.0,
            finish_update_cycle: 0.0,
            lts_submit: 0.0,
        }
    }

    fn zero(&mut self) {
        self.update_cycle = 0.0;
        self.zero_throughput_and_rtt = 0.0;
        self.copy_previous_and_reset_rtt = 0.0;
        self.apply_new_throughput_counters = 0.0;
        self.apply_flow_data = 0.0;
        self.apply_queue_stats = 0.0;
        self.update_totals = 0.0;
        self.next_cycle = 0.0;
        self.finish_update_cycle = 0.0;
        self.lts_submit = 0.0;
        self.start = Instant::now();
    }
}

fn throughput_task(
    netflow_sender: crossbeam_channel::Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
    system_usage_actor: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
    bakery_sender: crossbeam_channel::Sender<BakeryCommands>,
) {
    // Load RTT exclusion overrides once on startup. UI/API calls will refresh this on update.
    crate::rtt_exclusions::refresh_from_disk();

    // Obtain the flow timeout from the config, default to 30 seconds
    let timeout_seconds = if let Ok(config) = lqos_config::load_config() {
        if let Some(flow_config) = &config.flows {
            flow_config.flow_timeout_seconds
        } else {
            30
        }
    } else {
        30
    };

    let mut last_submitted_to_lts: Option<Instant> = None;
    let mut tfd = match TimerFd::new() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to create timer for throughput monitor: {}", e);
            return;
        }
    };
    assert_eq!(tfd.get_state(), TimerState::Disarmed);
    tfd.set_state(
        TimerState::Periodic {
            current: Duration::new(1, 0),
            interval: Duration::new(1, 0),
        },
        SetTimeFlags::Default,
    );
    let mut timer_metrics = ThroughputTaskTimeMetrics::new();

    // Preallocate some buffers to avoid allocations in the loop
    let mut rtt_circuit_tracker: FxHashMap<XdpIpAddress, RttBuffer> = FxHashMap::default();
    let mut rtt_by_circuit: FxHashMap<i64, RttBuffer> = FxHashMap::default();
    let mut tcp_retries: FxHashMap<XdpIpAddress, DownUpOrder<u64>> = FxHashMap::default();
    let mut tcp_retry_packets: FxHashMap<XdpIpAddress, DownUpOrder<u64>> = FxHashMap::default();
    let mut expired_flows: Vec<FlowbeeKey> = Vec::new();
    let mut finished_flow_exports: Vec<FinishedFlowExport> = Vec::new();

    // Counter for occasional stats
    let mut stats_counter = 0;
    let mut reload_backoff_logged = false;
    let mut last_reload_poll =
        Instant::now() - Duration::from_secs(RELOAD_THROUGHPUT_POLL_INTERVAL_SECONDS);

    loop {
        let start = Instant::now();
        timer_metrics.zero();
        let bakery_reload_in_progress = full_reload_in_progress();
        if bakery_reload_in_progress {
            if !reload_backoff_logged {
                info!(
                    "Throughput monitor: backing off to every {} seconds while Bakery full reload is in progress",
                    RELOAD_THROUGHPUT_POLL_INTERVAL_SECONDS
                );
                reload_backoff_logged = true;
            }
            if start.duration_since(last_reload_poll)
                < Duration::from_secs(RELOAD_THROUGHPUT_POLL_INTERVAL_SECONDS)
            {
                let missed_ticks = tfd.read();
                if missed_ticks > 1 {
                    warn!("Missed {} ticks", missed_ticks - 1);
                }
                continue;
            }
            last_reload_poll = start;
        } else if reload_backoff_logged {
            info!("Throughput monitor: resuming 1-second polling after Bakery full reload");
            reload_backoff_logged = false;
        }

        {
            lqos_network_devices::with_network_json_write(|net_json_calc| {
                timer_metrics.update_cycle = timer_metrics.start.elapsed().as_secs_f64();
                net_json_calc.zero_throughput_and_rtt();
                timer_metrics.zero_throughput_and_rtt = timer_metrics.start.elapsed().as_secs_f64();
                THROUGHPUT_TRACKER.copy_previous_and_reset_rtt();
                timer_metrics.copy_previous_and_reset_rtt =
                    timer_metrics.start.elapsed().as_secs_f64();
                THROUGHPUT_TRACKER
                    .apply_new_throughput_counters(net_json_calc, bakery_sender.clone());
                timer_metrics.apply_new_throughput_counters =
                    timer_metrics.start.elapsed().as_secs_f64();
                THROUGHPUT_TRACKER.apply_flow_data(FlowApplyContext {
                    timeout_seconds,
                    net_json_calc,
                    rtt_circuit_tracker: &mut rtt_circuit_tracker,
                    rtt_by_circuit: &mut rtt_by_circuit,
                    tcp_retries: &mut tcp_retries,
                    tcp_retry_packets: &mut tcp_retry_packets,
                    expired_keys: &mut expired_flows,
                    finished_flow_exports: &mut finished_flow_exports,
                });
                CIRCUIT_RTT_BUFFERS.store(Arc::new(rtt_by_circuit.clone()));
                THROUGHPUT_TRACKER.record_circuit_heatmaps();
                let enable_site_heatmaps = lqos_config::load_config()
                    .map(|config| config.enable_site_heatmaps)
                    .unwrap_or(true);
                net_json_calc.record_site_heatmaps(enable_site_heatmaps);

                timer_metrics.apply_flow_data = timer_metrics.start.elapsed().as_secs_f64();
                if bakery_reload_in_progress {
                    debug!(
                        "Throughput monitor: skipping queue-stat application during Bakery full reload"
                    );
                } else {
                    THROUGHPUT_TRACKER.apply_queue_stats(net_json_calc);
                }
                timer_metrics.apply_queue_stats = timer_metrics.start.elapsed().as_secs_f64();
                THROUGHPUT_TRACKER.update_totals();
                timer_metrics.update_totals = timer_metrics.start.elapsed().as_secs_f64();
                THROUGHPUT_TRACKER.next_cycle();
                timer_metrics.next_cycle = timer_metrics.start.elapsed().as_secs_f64();
            });

            dedup_flow_keys(&mut expired_flows);
            refresh_active_flow_snapshot();
            finish_expired_flows(
                &mut finished_flow_exports,
                expired_flows.as_mut_slice(),
                &netflow_sender,
                lqos_sys::end_flows,
            );

            // Clean up work tables after post-update exports and BPF cleanup.
            rtt_circuit_tracker.clear();
            rtt_by_circuit.clear();
            tcp_retries.clear();
            tcp_retry_packets.clear();
            expired_flows.clear();

            timer_metrics.finish_update_cycle = timer_metrics.start.elapsed().as_secs_f64();
            let duration_ms = start.elapsed().as_micros();
            TIME_TO_POLL_HOSTS.store(duration_ms as u64, std::sync::atomic::Ordering::Relaxed);
        }

        if last_submitted_to_lts.is_none() {
            stats_submission::submit_throughput_stats(
                1.0,
                stats_counter,
                system_usage_actor.clone(),
            );
        } else if let Some(last) = last_submitted_to_lts {
            let elapsed_f64 = last.elapsed().as_secs_f64();
            let my_system_usage_actor = system_usage_actor.clone();
            // Submit if a reasonable amount of time has passed - drop if there was a long hitch
            if elapsed_f64 < 2.0 {
                match std::thread::Builder::new()
                    .name("Throughput Stats Submit".to_string())
                    .spawn(move || {
                        stats_submission::submit_throughput_stats(
                            elapsed_f64,
                            stats_counter,
                            my_system_usage_actor,
                        );
                    }) {
                    Ok(handle) => {
                        if let Err(e) = handle.join() {
                            info!(
                                "Throughput stats submit thread join error (ignored): {:?}",
                                e
                            );
                        }
                    }
                    Err(e) => {
                        info!(
                            "Failed to spawn throughput stats submit thread (ignored): {:?}",
                            e
                        );
                    }
                }
            }
        } else {
            info!("No last submission timestamp; skipping stats submission this cycle");
        }
        // Notify of completion, which triggers processing
        if let Err(e) = crate::lts2_sys::ingest_batch_complete() {
            tracing::log::warn!("Error sending message to LTS2: {e:?}");
        }
        last_submitted_to_lts = Some(Instant::now());
        timer_metrics.lts_submit = timer_metrics.start.elapsed().as_secs_f64();

        // Counter for occasional stats
        stats_counter = stats_counter.wrapping_add(1);

        // Sleep until the next second
        let missed_ticks = tfd.read();
        if missed_ticks > 1 {
            warn!("Missed {} ticks", missed_ticks - 1);
            warn!("{:?}", timer_metrics);
        }
    }
}

pub fn current_throughput() -> BusResponse {
    let (bits_per_second, packets_per_second, shaped_bits_per_second, tcp_pps, udp_pps, icmp_pps) = {
        (
            THROUGHPUT_TRACKER.actual_bits_per_second(),
            THROUGHPUT_TRACKER.packets_per_second(),
            THROUGHPUT_TRACKER.shaped_actual_bits_per_second(),
            THROUGHPUT_TRACKER.tcp_packets_per_second(),
            THROUGHPUT_TRACKER.udp_packets_per_second(),
            THROUGHPUT_TRACKER.icmp_packets_per_second(),
        )
    };
    BusResponse::CurrentThroughput {
        bits_per_second,
        packets_per_second,
        shaped_bits_per_second,
        tcp_packets_per_second: tcp_pps,
        udp_packets_per_second: udp_pps,
        icmp_packets_per_second: icmp_pps,
    }
}

pub fn host_counters() -> BusResponse {
    let mut result = Vec::new();
    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .for_each(|(k, v)| {
            let ip = k.as_ip();
            result.push((ip, v.bytes_per_second));
        });
    BusResponse::HostCounters(result)
}

#[inline(always)]
pub(crate) fn retire_check(cycle: u64, recent_cycle: u64) -> bool {
    cycle < recent_cycle + RETIRE_AFTER_SECONDS
}

type TopList = (
    XdpIpAddress,
    DownUpOrder<u64>,
    DownUpOrder<u64>,
    f32,
    TcHandle,
    String,
    DownUpOrder<TcpRetransmitSample>,
);

pub fn top_n(start: u32, end: u32) -> BusResponse {
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        let catalog = lqos_network_devices::network_devices_catalog();
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .map(|(k, te)| {
                let (circuit_id, _circuit_name) =
                    resolve_circuit_metadata_for_entry(&catalog, k, te);
                (
                    *k,
                    te.actual_bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    circuit_id,
                    down_up_retransmit_sample(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    full_list.sort_by_key(|row| std::cmp::Reverse(row.1.down));
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, circuit_id, tcp_retransmits)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: *tcp_retransmits,
            },
        )
        .collect();
    BusResponse::TopDownloaders(result)
}

pub fn top_n_up(start: u32, end: u32) -> BusResponse {
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        let catalog = lqos_network_devices::network_devices_catalog();
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .map(|(k, te)| {
                let (circuit_id, _circuit_name) =
                    resolve_circuit_metadata_for_entry(&catalog, k, te);
                (
                    *k,
                    te.actual_bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    circuit_id,
                    down_up_retransmit_sample(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    full_list.sort_by_key(|row| std::cmp::Reverse(row.1.up));
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, circuit_id, tcp_retransmits)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: *tcp_retransmits,
            },
        )
        .collect();
    BusResponse::TopUploaders(result)
}

/// Retrieve per-circuit heatmap data for the executive summary.
pub fn circuit_heatmaps() -> BusResponse {
    let enabled = lqos_config::load_config()
        .map(|cfg| cfg.enable_circuit_heatmaps)
        .unwrap_or(true);
    if !enabled {
        return BusResponse::CircuitHeatmaps(Vec::new());
    }

    let catalog = lqos_network_devices::network_devices_catalog();
    let mut circuit_meta: FxHashMap<i64, (String, String)> = FxHashMap::default();
    catalog.iter_all_devices().for_each(|device| {
        circuit_meta
            .entry(device.circuit_hash)
            .or_insert_with(|| (device.circuit_id.clone(), device.circuit_name.clone()));
    });

    let heatmaps = THROUGHPUT_TRACKER.circuit_heatmaps.lock();
    let qoq_heatmaps = THROUGHPUT_TRACKER.circuit_qoq_heatmaps.lock();
    let mut rows: Vec<CircuitHeatmapData> = heatmaps
        .iter()
        .map(|(hash, heatmap)| {
            let (circuit_id, circuit_name) = circuit_meta
                .get(hash)
                .cloned()
                .unwrap_or_else(|| (String::new(), String::new()));
            CircuitHeatmapData {
                circuit_hash: *hash,
                circuit_id,
                circuit_name,
                blocks: heatmap.blocks(),
                qoq_blocks: qoq_heatmaps.get(hash).map(|heatmap| heatmap.blocks()),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.circuit_id.cmp(&b.circuit_id));
    BusResponse::CircuitHeatmaps(rows)
}

/// Retrieve per-site heatmap data for the executive summary.
pub fn site_heatmaps() -> BusResponse {
    let enabled = lqos_config::load_config()
        .map(|cfg| cfg.enable_site_heatmaps)
        .unwrap_or(true);
    if !enabled {
        return BusResponse::SiteHeatmaps(Vec::new());
    }

    lqos_network_devices::with_network_json_read(|net_json| {
        let mut rows: Vec<SiteHeatmapData> = net_json
            .get_nodes_when_ready()
            .iter()
            .filter_map(|node| {
                if node.name == "Root" || node.name.parse::<std::net::IpAddr>().is_ok() {
                    return None;
                }
                node.heatmap.as_ref().map(|heatmap| SiteHeatmapData {
                    site_name: node.name.clone(),
                    node_type: node.node_type.clone(),
                    depth: node.parents.len().saturating_sub(1),
                    blocks: heatmap.blocks(),
                    qoq_blocks: node.qoq_heatmap.as_ref().map(|heatmap| heatmap.blocks()),
                })
            })
            .collect();
        rows.sort_by(|a, b| a.site_name.cmp(&b.site_name));
        BusResponse::SiteHeatmaps(rows)
    })
}

/// Retrieve per-ASN heatmap data for the executive summary.
pub fn asn_heatmaps() -> BusResponse {
    let enabled = lqos_config::load_config()
        .map(|cfg| cfg.enable_asn_heatmaps)
        .unwrap_or(true);
    if !enabled {
        return BusResponse::AsnHeatmaps(Vec::new());
    }

    let rows: Vec<AsnHeatmapData> = snapshot_asn_heatmaps()
        .into_iter()
        .map(|(asn, blocks)| {
            let name = get_asn_name_by_id(asn);
            let asn_name = if name.eq_ignore_ascii_case("unknown") {
                None
            } else {
                Some(name)
            };
            AsnHeatmapData {
                asn,
                asn_name,
                blocks,
            }
        })
        .collect();
    BusResponse::AsnHeatmaps(rows)
}

/// Retrieve the global roll-up heatmap data for the executive summary.
pub fn global_heatmap() -> BusResponse {
    let heatmap = THROUGHPUT_TRACKER.global_heatmap.lock();
    BusResponse::GlobalHeatmap(heatmap.blocks())
}

pub fn worst_n(start: u32, end: u32) -> BusResponse {
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        let catalog = lqos_network_devices::network_devices_catalog();
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(_k, d)| {
                d.circuit_hash
                    .map(|h| !crate::rtt_exclusions::is_excluded_hash(h))
                    .unwrap_or(true)
            })
            .filter(|(_k, te)| te.median_latency().is_some())
            .map(|(k, te)| {
                let (circuit_id, _circuit_name) =
                    resolve_circuit_metadata_for_entry(&catalog, k, te);
                (
                    *k,
                    te.actual_bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    circuit_id,
                    down_up_retransmit_sample(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    full_list.sort_by(|a, b| b.3.total_cmp(&a.3));
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, circuit_id, tcp_retransmits)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: *tcp_retransmits,
            },
        )
        .collect();
    BusResponse::WorstRtt(result)
}

pub fn worst_n_retransmits(start: u32, end: u32) -> BusResponse {
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        let catalog = lqos_network_devices::network_devices_catalog();
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(_k, te)| te.median_latency().is_some())
            .map(|(k, te)| {
                let (circuit_id, _circuit_name) =
                    resolve_circuit_metadata_for_entry(&catalog, k, te);
                (
                    *k,
                    te.actual_bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    circuit_id,
                    down_up_retransmit_sample(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    // Use a total order for floating-point comparison to avoid panics
    // when NaN/Inf are present and ensure comparator transitivity.
    full_list.sort_by(|a, b| {
        let total_a = a.6.down.fraction().map(|f| f.get()).unwrap_or(0.0)
            + a.6.up.fraction().map(|f| f.get()).unwrap_or(0.0);
        let total_b = b.6.down.fraction().map(|f| f.get()).unwrap_or(0.0)
            + b.6.up.fraction().map(|f| f.get()).unwrap_or(0.0);
        total_b.total_cmp(&total_a)
    });
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, circuit_id, tcp_retransmits)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: *tcp_retransmits,
            },
        )
        .collect();
    BusResponse::WorstRetransmits(result)
}

pub fn best_n(start: u32, end: u32) -> BusResponse {
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        let catalog = lqos_network_devices::network_devices_catalog();
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(_k, d)| {
                d.circuit_hash
                    .map(|h| !crate::rtt_exclusions::is_excluded_hash(h))
                    .unwrap_or(true)
            })
            .filter(|(_k, te)| te.median_latency().is_some())
            .map(|(k, te)| {
                let (circuit_id, _circuit_name) =
                    resolve_circuit_metadata_for_entry(&catalog, k, te);
                (
                    *k,
                    te.actual_bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    circuit_id,
                    down_up_retransmit_sample(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    full_list.sort_by(|a, b| b.3.total_cmp(&a.3));
    full_list.reverse();
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, circuit_id, tcp_retransmits)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: *tcp_retransmits,
            },
        )
        .collect();
    BusResponse::BestRtt(result)
}

pub fn xdp_pping_compat() -> BusResponse {
    let raw_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);
    let result = THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .filter(|(_k, d)| retire_check(raw_cycle, d.most_recent_cycle))
        .filter_map(|(_k, data)| {
            if data.tc_handle.as_u32() > 0 {
                let mut valid_samples: Vec<u32> = data
                    .recent_rtt_data
                    .iter()
                    .filter(|d| d.as_millis_times_100() > 0.0)
                    .map(|d| d.as_millis_times_100() as u32)
                    .collect();
                let samples = valid_samples.len() as u32;
                if samples > 0 {
                    valid_samples.sort_by(|a, b| (*a).cmp(b));
                    let median = valid_samples[valid_samples.len() / 2] as f32 / 100.0;
                    let min = if let Some(v) = valid_samples.first() {
                        *v as f32 / 100.0
                    } else {
                        // No valid min; skip this submission as if no samples
                        return None;
                    };
                    let max = if let Some(v) = valid_samples.last() {
                        *v as f32 / 100.0
                    } else {
                        // No valid max; skip this submission as if no samples
                        return None;
                    };
                    let sum = valid_samples.iter().sum::<u32>() as f32 / 100.0;
                    let avg = sum / samples as f32;

                    Some(XdpPpingResult {
                        tc: data.tc_handle.to_string(),
                        median,
                        avg,
                        max,
                        min,
                        samples,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();
    BusResponse::XdpPping(result)
}

pub struct MinMaxMedianRtt {
    pub min: f32,
    pub max: f32,
    pub median: f32,
}

pub fn min_max_median_rtt() -> Option<MinMaxMedianRtt> {
    let reader_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);

    // Put all valid RTT samples into a big buffer
    let mut samples: Vec<f32> = Vec::new();

    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .filter(|(_k, d)| retire_check(reader_cycle, d.most_recent_cycle))
        .for_each(|(_k, d)| {
            samples.extend(
                d.recent_rtt_data
                    .iter()
                    .filter(|d| d.as_millis() > 0.0)
                    .map(|d| d.as_millis() as f32)
                    .collect::<Vec<f32>>(),
            );
        });

    if samples.is_empty() {
        return None;
    }

    // Sort the buffer
    samples.sort_by(|a, b| a.total_cmp(b));

    let result = MinMaxMedianRtt {
        min: samples[0],
        max: samples[samples.len() - 1],
        median: samples[samples.len() / 2],
    };

    Some(result)
}

#[derive(Debug, Serialize)]
pub struct TcpRetransmitTotal {
    pub up: i32,
    pub down: i32,
    pub tcp_up: u64,
    pub tcp_down: u64,
}

pub fn min_max_median_tcp_retransmits() -> TcpRetransmitTotal {
    let reader_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);

    let total_tcp = THROUGHPUT_TRACKER.tcp_packets_per_second();
    let mut total = TcpRetransmitTotal {
        up: 0,
        down: 0,
        tcp_down: total_tcp.down,
        tcp_up: total_tcp.up,
    };

    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .filter(|(_k, d)| retire_check(reader_cycle, d.most_recent_cycle))
        .for_each(|(_k, d)| {
            total.up += d.tcp_retransmits.up as i32;
            total.down += d.tcp_retransmits.down as i32;
        });

    total
}

pub fn rtt_histogram<const N: usize>() -> BusResponse {
    let mut result = vec![0; N];
    let reader_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);
    for (_k, data) in THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .filter(|(_k, d)| retire_check(reader_cycle, d.most_recent_cycle))
    {
        let valid_samples: Vec<f64> = data
            .recent_rtt_data
            .iter()
            .filter(|d| d.as_millis() > 0.0)
            .map(|d| d.as_millis())
            .collect();
        let samples = valid_samples.len() as u32;
        if samples > 0 {
            let median = valid_samples[valid_samples.len() / 2] as f32 / 10.0;
            let median = f32::min(N as f32 * 10.0, median);
            let column = median as usize;
            result[usize::min(column, N - 1)] += 1;
        }
    }

    BusResponse::RttHistogram(result)
}

pub fn host_counts() -> BusResponse {
    let (total, shaped) = current_host_counts();
    BusResponse::HostCounts((total, shaped))
}

fn current_host_counts() -> (u32, u32) {
    let mut total = 0;
    let mut shaped = 0;
    let tp_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);
    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .iter()
        .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
        .for_each(|(_k, d)| {
            total += 1;
            if d.tc_handle.as_u32() != 0 {
                shaped += 1;
            }
        });
    (total, shaped)
}

/// Gather headline metrics for the Executive Summary header cards.
pub fn executive_summary_header() -> BusResponse {
    let catalog = lqos_network_devices::network_devices_catalog();
    let mut circuits: FxHashSet<i64> = FxHashSet::default();
    let mut device_count = 0_u64;
    for device in catalog.iter_all_devices() {
        device_count = device_count.saturating_add(1);
        circuits.insert(device.circuit_hash);
    }
    let circuit_count = circuits.len() as u64;

    let site_count = lqos_network_devices::with_network_json_read(|net_json| {
        let total_nodes = net_json.get_nodes_when_ready().len();
        // Remove the synthetic root node when counting sites.
        total_nodes.saturating_sub(1) as u64
    });

    let (total_hosts, shaped_hosts) = current_host_counts();
    let mapped_ip_count = shaped_hosts as u64;
    let unmapped_ip_count = total_hosts.saturating_sub(shaped_hosts) as u64;

    let queue_counts = ALL_QUEUE_SUMMARY.queue_counts();
    let bakery_reload_in_progress = full_reload_in_progress();
    let queue_stats_stale = queue_stats_stale() || bakery_reload_in_progress;
    let insight_connected = crate::lts2_sys::current_capabilities().control_service_reachable;

    BusResponse::ExecutiveSummaryHeader(ExecutiveSummaryHeader {
        circuit_count,
        device_count,
        site_count,
        mapped_ip_count,
        unmapped_ip_count,
        htb_queue_count: queue_counts.htb as u64,
        cake_queue_count: queue_counts.cake as u64,
        fq_codel_queue_count: queue_counts.fq_codel as u64,
        queue_stats_stale,
        bakery_reload_in_progress,
        insight_connected,
    })
}

type FullList = (
    XdpIpAddress,
    DownUpOrder<u64>,
    DownUpOrder<u64>,
    f32,
    TcHandle,
    u64,
);

pub fn all_unknown_ips() -> BusResponse {
    let boot_time = time_since_boot();
    if boot_time.is_err() {
        warn!("The Linux system clock isn't available to provide time since boot, yet.");
        warn!("This only happens immediately after a reboot.");
        return BusResponse::NotReadyYet;
    }
    let Ok(boot_time) = boot_time else {
        return BusResponse::Fail("Boot time unavailable".to_string());
    };

    // Safely convert TimeSpec to Duration - handle potential negative values
    let time_since_boot = match boot_time.tv_sec() {
        sec if sec < 0 => {
            warn!(
                "Negative boot time detected: {:?}. Using 0 duration.",
                boot_time
            );
            Duration::from_secs(0)
        }
        sec => Duration::from_secs(sec as u64) + Duration::from_nanos(boot_time.tv_nsec() as u64),
    };

    let five_minutes_ago = time_since_boot.saturating_sub(Duration::from_secs(300));
    let five_minutes_ago_nanoseconds = five_minutes_ago.as_nanos();

    let mut full_list: Vec<FullList> = {
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| d.tc_handle.as_u32() == 0)
            .filter(|(_k, d)| d.last_seen as u128 > five_minutes_ago_nanoseconds)
            .map(|(k, te)| {
                (
                    *k,
                    te.bytes,
                    te.packets,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    te.most_recent_cycle,
                )
            })
            .collect()
    };
    full_list.sort_by_key(|row| std::cmp::Reverse(row.5));
    let result = full_list
        .iter()
        .map(
            |(ip, bytes, packets, median_rtt, tc_handle, _last_seen)| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: String::new(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmit_sample: DownUpOrder::new(
                    TcpRetransmitSample::new(0, 0),
                    TcpRetransmitSample::new(0, 0),
                ),
            },
        )
        .collect();
    BusResponse::AllUnknownIps(result)
}

fn flow_summary_from_snapshot(flow: &ActiveFlowSnapshot) -> lqos_bus::FlowbeeSummaryData {
    lqos_bus::FlowbeeSummaryData {
        remote_ip: flow.display.remote_ip.clone(),
        local_ip: flow.display.local_ip.clone(),
        src_port: flow.display.src_port,
        dst_port: flow.display.dst_port,
        ip_protocol: flow.display.ip_protocol.clone(),
        bytes_sent: flow.bytes_sent,
        packets_sent: flow.packets_sent,
        rate_estimate_bps: flow.rate_estimate_bps,
        tcp_retransmits: flow.tcp_retransmits,
        end_status: flow.end_status,
        tos: flow.tos,
        flags: flow.flags,
        remote_asn: flow.display.remote_asn,
        remote_asn_name: flow.display.remote_asn_name.clone(),
        remote_asn_country: flow.display.remote_asn_country.clone(),
        analysis: flow.display.analysis.clone(),
        last_seen: flow.last_seen,
        start_time: flow.start_time,
        rtt_nanos: flow.rtt_nanos,
        circuit_id: flow.circuit_id.clone(),
        circuit_name: flow.circuit_name.clone(),
    }
}

/// Returns all rows from the latest active-flow snapshot.
pub fn dump_active_flows() -> BusResponse {
    let snapshot = active_flow_snapshot();
    let result: Vec<lqos_bus::FlowbeeSummaryData> =
        snapshot.iter().map(flow_summary_from_snapshot).collect();

    BusResponse::AllActiveFlows(result)
}

/// Count active flows
pub fn count_active_flows() -> BusResponse {
    BusResponse::CountActiveFlows(live_active_flow_count())
}

fn compare_top_flow(
    a: &ActiveFlowSnapshot,
    b: &ActiveFlowSnapshot,
    flow_type: &TopFlowType,
) -> Ordering {
    match flow_type {
        TopFlowType::RateEstimate => b.rate_estimate_bps.sum().cmp(&a.rate_estimate_bps.sum()),
        TopFlowType::Bytes => b.bytes_sent.sum().cmp(&a.bytes_sent.sum()),
        TopFlowType::Packets => b.packets_sent.sum().cmp(&a.packets_sent.sum()),
        TopFlowType::Drops => b.tcp_retransmits.sum().cmp(&a.tcp_retransmits.sum()),
        TopFlowType::RoundTripTime => a.rtt_nanos.down.cmp(&b.rtt_nanos.down),
    }
}

/// Top Flows Report
pub fn top_flows(n: u32, flow_type: TopFlowType) -> BusResponse {
    let snapshot = active_flow_snapshot();
    let mut table: Vec<&ActiveFlowSnapshot> = snapshot.iter().collect();
    let limit = n as usize;

    if limit == 0 {
        table.clear();
    } else if limit < table.len() {
        table.select_nth_unstable_by(limit, |a, b| compare_top_flow(a, b, &flow_type));
        table.truncate(limit);
    }
    table.sort_by(|a, b| compare_top_flow(a, b, &flow_type));

    let result = table
        .iter()
        .map(|flow| flow_summary_from_snapshot(flow))
        .collect();

    BusResponse::TopFlows(result)
}

/// Flows by IP
pub fn flows_by_ip(ip: &str) -> BusResponse {
    if let Ok(ip) = ip.parse::<IpAddr>() {
        let ip = XdpIpAddress::from_ip(ip);
        let snapshot = active_flow_snapshot();
        let matching_flows: Vec<_> = snapshot
            .iter()
            .filter(|flow| flow.key.local_ip == ip)
            .map(flow_summary_from_snapshot)
            .collect();

        return BusResponse::FlowsByIp(matching_flows);
    }
    BusResponse::Ack
}

/// Current endpoints by country
pub fn current_endpoints_by_country() -> BusResponse {
    let summary = flow_data::RECENT_FLOWS.country_summary();
    BusResponse::CurrentEndpointsByCountry(summary)
}

/// Current endpoint lat/lon
pub fn current_lat_lon() -> BusResponse {
    let summary = flow_data::RECENT_FLOWS.lat_lon_endpoints();
    BusResponse::CurrentLatLon(summary)
}

/// Ether Protocol Summary
pub fn ether_protocol_summary() -> BusResponse {
    flow_data::RECENT_FLOWS.ether_protocol_summary()
}

/// IP Protocol Summary
pub fn ip_protocol_summary() -> BusResponse {
    BusResponse::IpProtocols(flow_data::RECENT_FLOWS.ip_protocol_summary())
}

/// Flow duration summary
pub fn flow_duration() -> BusResponse {
    BusResponse::FlowDuration(
        flow_data::RECENT_FLOWS
            .flow_duration_summary()
            .into_iter()
            .map(|v| (v.count, v.duration))
            .collect(),
    )
}

type RawNetJs = std::collections::HashMap<String, RawNetJsBody>;

#[derive(Deserialize, Debug)]
struct RawNetJsBody {
    #[serde(rename = "downloadBandwidthMbps")]
    download_bandwidth_mbps: u32,
    #[serde(rename = "uploadBandwidthMbps")]
    upload_bandwidth_mbps: u32,
    #[serde(default)]
    latitude: Option<f32>,
    #[serde(default)]
    longitude: Option<f32>,
    #[serde(rename = "type")]
    site_type: Option<String>,
    children: Option<RawNetJs>,
}

#[derive(Serialize, Debug)]
struct Lts2NetJs {
    name: String,
    site_hash: i64,
    site_type: Option<String>,
    download_bandwidth_mbps: u32,
    upload_bandwidth_mbps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    latitude: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    longitude: Option<f32>,
    children: Vec<Lts2NetJs>,
}

impl RawNetJsBody {
    fn to_lts2(&self, name: &str) -> Lts2NetJs {
        let mut result = Lts2NetJs {
            name: name.to_string(),
            site_hash: hash_to_i64(name),
            site_type: self.site_type.clone(),
            download_bandwidth_mbps: self.download_bandwidth_mbps,
            upload_bandwidth_mbps: self.upload_bandwidth_mbps,
            latitude: self.latitude,
            longitude: self.longitude,
            children: vec![],
        };

        if let Some(children) = &self.children {
            for (name, body) in children.iter() {
                result.children.push(body.to_lts2(name));
            }
        }

        result
    }
}

#[repr(C)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lts2Circuit {
    pub circuit_id: String,
    pub circuit_name: String,
    pub circuit_hash: i64,
    pub download_min_mbps: u32,
    pub upload_min_mbps: u32,
    pub download_max_mbps: u32,
    pub upload_max_mbps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_min_mbps_exact: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_min_mbps_exact: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_max_mbps_exact: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_max_mbps_exact: Option<f32>,
    pub parent_node: i64,
    pub parent_node_name: Option<String>,
    pub devices: Vec<Lts2Device>,
}

#[repr(C)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lts2Device {
    pub device_id: String,
    pub device_name: String,
    pub device_hash: i64,
    pub mac: String,
    pub ipv4: Vec<([u8; 4], u8)>,
    pub ipv6: Vec<([u8; 16], u8)>,
    pub comment: String,
}

#[cfg(test)]
mod compatibility_tests {
    use super::{
        CIRCUIT_RTT_BUFFERS, Lts2Circuit, RawNetJsBody, circuit_current_qoo,
        circuit_current_rtt_p50_nanos, finish_expired_flows, resolve_circuit_metadata_for_entry,
        resolve_flow_device,
    };
    use crate::test_support::ActiveFlowSnapshotTestContext;
    use crate::throughput_tracker::flow_data::{
        FlowAnalysis, FlowbeeEffectiveDirection, FlowbeeLocalData, RttData, active_flow_snapshot,
        active_flow_test_lock, mutate_all_flows, refresh_active_flow_snapshot,
        replace_active_flows_live_for_test,
    };
    use crate::throughput_tracker::throughput_entry::ThroughputEntry;
    use fxhash::FxHashMap;
    use lqos_bus::{BusResponse, TcHandle, TopFlowType};
    use lqos_config::{ConfigShapedDevices, ShapedDevice};
    use lqos_network_devices::{NetworkDevicesCatalog, ShapedDevicesCatalog};
    use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
    use lqos_utils::{XdpIpAddress, hash_to_i64};
    use lqos_utils::qoo::QoqScores;
    use lqos_utils::qoq_heatmap::TemporalQoqHeatmap;
    use lqos_utils::rtt::RttBuffer;
    use lqos_utils::units::DownUpOrder;
    use serde::{Deserialize, Serialize};
    use serde_json::to_value;
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    #[allow(dead_code)]
    #[derive(Debug, Deserialize, Serialize)]
    struct OldLts2Device {
        device_id: String,
        device_name: String,
        device_hash: i64,
        mac: String,
        ipv4: Vec<([u8; 4], u8)>,
        ipv6: Vec<([u8; 16], u8)>,
        comment: String,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize, Serialize)]
    struct OldLts2Circuit {
        circuit_id: String,
        circuit_name: String,
        circuit_hash: i64,
        download_min_mbps: u32,
        upload_min_mbps: u32,
        download_max_mbps: u32,
        upload_max_mbps: u32,
        parent_node: i64,
        parent_node_name: Option<String>,
        devices: Vec<OldLts2Device>,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct OldLts2NetJs {
        name: String,
        site_hash: i64,
        site_type: Option<String>,
        download_bandwidth_mbps: u32,
        upload_bandwidth_mbps: u32,
        children: Vec<OldLts2NetJs>,
    }

    #[test]
    fn old_receivers_ignore_exact_rate_fields() {
        let current = Lts2Circuit {
            circuit_id: "cid".to_string(),
            circuit_name: "Circuit".to_string(),
            circuit_hash: 7,
            download_min_mbps: 3,
            upload_min_mbps: 2,
            download_max_mbps: 7,
            upload_max_mbps: 4,
            download_min_mbps_exact: Some(2.5),
            upload_min_mbps_exact: Some(1.5),
            download_max_mbps_exact: Some(6.6),
            upload_max_mbps_exact: Some(3.3),
            parent_node: 9,
            parent_node_name: Some("Parent".to_string()),
            devices: Vec::new(),
        };

        let bytes = serde_cbor::to_vec(&current).expect("current payload serializes");
        let decoded: OldLts2Circuit =
            serde_cbor::from_slice(&bytes).expect("legacy shape ignores extra fields");

        assert_eq!(decoded.download_max_mbps, 7);
        assert_eq!(decoded.upload_max_mbps, 4);
    }

    #[test]
    fn current_lts2_circuit_round_trips_exact_rate_fields() {
        let current = Lts2Circuit {
            circuit_id: "cid".to_string(),
            circuit_name: "Circuit".to_string(),
            circuit_hash: 7,
            download_min_mbps: 3,
            upload_min_mbps: 2,
            download_max_mbps: 7,
            upload_max_mbps: 4,
            download_min_mbps_exact: Some(2.5),
            upload_min_mbps_exact: Some(1.5),
            download_max_mbps_exact: Some(6.6),
            upload_max_mbps_exact: Some(3.3),
            parent_node: 9,
            parent_node_name: Some("Parent".to_string()),
            devices: Vec::new(),
        };

        let bytes = serde_cbor::to_vec(&current).expect("current payload serializes");
        let decoded: Lts2Circuit =
            serde_cbor::from_slice(&bytes).expect("current payload round trips");

        assert_eq!(decoded.download_min_mbps_exact, Some(2.5));
        assert_eq!(decoded.upload_min_mbps_exact, Some(1.5));
        assert_eq!(decoded.download_max_mbps_exact, Some(6.6));
        assert_eq!(decoded.upload_max_mbps_exact, Some(3.3));
    }

    #[test]
    fn current_receivers_default_missing_exact_rate_fields() {
        let old = OldLts2Circuit {
            circuit_id: "cid".to_string(),
            circuit_name: "Circuit".to_string(),
            circuit_hash: 7,
            download_min_mbps: 3,
            upload_min_mbps: 2,
            download_max_mbps: 7,
            upload_max_mbps: 4,
            parent_node: 9,
            parent_node_name: Some("Parent".to_string()),
            devices: Vec::new(),
        };

        let bytes = serde_cbor::to_vec(&old).expect("legacy payload serializes");
        let decoded: Lts2Circuit =
            serde_cbor::from_slice(&bytes).expect("current shape accepts missing exact fields");

        assert_eq!(decoded.download_max_mbps, 7);
        assert_eq!(decoded.upload_max_mbps, 4);
        assert_eq!(decoded.download_min_mbps_exact, None);
        assert_eq!(decoded.upload_min_mbps_exact, None);
        assert_eq!(decoded.download_max_mbps_exact, None);
        assert_eq!(decoded.upload_max_mbps_exact, None);
    }

    #[test]
    fn finish_expired_flows_sends_exports_and_invokes_cleanup_once() {
        let mut key = FlowbeeKey::default();
        key.ip_protocol = 6;
        key.src_port = 443;
        key.dst_port = 54_321;

        let mut raw = FlowbeeData::default();
        raw.start_time = 10;
        raw.last_seen = 20;
        raw.bytes_sent = DownUpOrder::new(1_000, 2_000);
        raw.packets_sent = DownUpOrder::new(10, 20);
        raw.rate_estimate_bps = DownUpOrder::new(30_000, 40_000);
        raw.circuit_hash = 123;
        raw.device_hash = 456;

        let export = (
            key,
            (
                FlowbeeLocalData::from_flow(&raw, &key),
                FlowAnalysis::new(&key),
            ),
        );
        let (sender, receiver) = crossbeam_channel::unbounded();
        let mut finished_flow_exports = vec![export];
        let mut expired_flows = vec![key];
        let mut cleanup_keys = Vec::new();

        finish_expired_flows(
            &mut finished_flow_exports,
            expired_flows.as_mut_slice(),
            &sender,
            |keys| {
                cleanup_keys.extend_from_slice(keys);
                Ok(())
            },
        );

        let received = receiver
            .try_recv()
            .expect("finished flow export should be sent");
        assert_eq!(received.0, key);
        assert!(receiver.try_recv().is_err());
        assert!(finished_flow_exports.is_empty());
        assert_eq!(expired_flows, vec![key]);
        assert_eq!(cleanup_keys, vec![key]);
    }

    #[test]
    fn finish_expired_flows_runs_cleanup_when_export_receiver_is_closed() {
        let mut key = FlowbeeKey::default();
        key.ip_protocol = 6;
        key.src_port = 443;
        key.dst_port = 54_320;

        let mut raw = FlowbeeData::default();
        raw.start_time = 10;
        raw.last_seen = 20;
        raw.bytes_sent = DownUpOrder::new(1_000, 2_000);
        raw.packets_sent = DownUpOrder::new(10, 20);
        raw.rate_estimate_bps = DownUpOrder::new(30_000, 40_000);

        let (sender, receiver) = crossbeam_channel::unbounded();
        drop(receiver);
        let mut finished_flow_exports = vec![(
            key,
            (
                FlowbeeLocalData::from_flow(&raw, &key),
                FlowAnalysis::new(&key),
            ),
        )];
        let mut expired_flows = vec![key];
        let mut cleanup_keys = Vec::new();

        finish_expired_flows(
            &mut finished_flow_exports,
            expired_flows.as_mut_slice(),
            &sender,
            |keys| {
                cleanup_keys.extend_from_slice(keys);
                Ok(())
            },
        );

        assert!(finished_flow_exports.is_empty());
        assert_eq!(cleanup_keys, vec![key]);
    }

    #[test]
    fn dedup_flow_keys_keeps_first_seen_order() {
        let mut key_1 = FlowbeeKey::default();
        key_1.ip_protocol = 6;
        key_1.src_port = 443;
        key_1.dst_port = 54_310;
        let mut key_2 = key_1;
        key_2.dst_port = 54_311;
        let mut key_3 = key_1;
        key_3.dst_port = 54_312;
        let mut keys = vec![key_1, key_2, key_1, key_3, key_2];

        super::dedup_flow_keys(&mut keys);

        assert_eq!(keys, vec![key_1, key_2, key_3]);
    }

    #[test]
    fn post_write_snapshot_refresh_publishes_live_writes() {
        let _guard = active_flow_test_lock();
        let mut key = FlowbeeKey::default();
        key.ip_protocol = 6;
        key.src_port = 443;
        key.dst_port = 54_322;

        let mut raw = FlowbeeData::default();
        raw.start_time = 10;
        raw.last_seen = 20;
        raw.bytes_sent = DownUpOrder::new(3_000, 4_000);
        raw.packets_sent = DownUpOrder::new(30, 40);
        raw.rate_estimate_bps = DownUpOrder::new(50_000, 60_000);

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
        assert!(active_flow_snapshot().is_empty());

        mutate_all_flows(|flows| {
            flows.insert(
                key,
                (
                    FlowbeeLocalData::from_flow(&raw, &key),
                    FlowAnalysis::new(&key),
                ),
            );
        });
        assert!(active_flow_snapshot().is_empty());

        refresh_active_flow_snapshot();

        let snapshot = active_flow_snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].key, key);

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
    }

    #[test]
    fn active_flow_bus_readers_use_published_snapshot_fields() {
        let _guard = active_flow_test_lock();
        let local_ip_1 = XdpIpAddress::from_ip("192.0.2.10".parse().expect("test IP should parse"));
        let local_ip_2 = XdpIpAddress::from_ip("192.0.2.11".parse().expect("test IP should parse"));
        let remote_ip_1 =
            XdpIpAddress::from_ip("198.51.100.20".parse().expect("test IP should parse"));
        let remote_ip_2 =
            XdpIpAddress::from_ip("198.51.100.21".parse().expect("test IP should parse"));

        let mut key_1 = FlowbeeKey::default();
        key_1.local_ip = local_ip_1;
        key_1.remote_ip = remote_ip_1;
        key_1.ip_protocol = 6;
        key_1.src_port = 443;
        key_1.dst_port = 50_000;

        let mut key_2 = key_1;
        key_2.local_ip = local_ip_2;
        key_2.remote_ip = remote_ip_2;
        key_2.dst_port = 50_001;

        let mut raw_1 = FlowbeeData::default();
        raw_1.start_time = 10_000_000_000;
        raw_1.last_seen = 20_000_000_000;
        raw_1.bytes_sent = DownUpOrder::new(1_000, 2_000);
        raw_1.packets_sent = DownUpOrder::new(10, 20);
        raw_1.rate_estimate_bps = DownUpOrder::new(30_000, 40_000);
        raw_1.tcp_retransmits = DownUpOrder::new(1, 2);
        raw_1.tos = 4;
        raw_1.flags = 0x12;
        raw_1.circuit_hash = 9_001;
        raw_1.device_hash = 9_002;

        let mut raw_2 = raw_1.clone();
        raw_2.bytes_sent = DownUpOrder::new(10_000, 20_000);
        raw_2.packets_sent = DownUpOrder::new(100, 200);
        raw_2.rate_estimate_bps = DownUpOrder::new(300_000, 400_000);
        raw_2.tcp_retransmits = DownUpOrder::new(9, 9);

        mutate_all_flows(|flows| {
            flows.clear();
            let mut local_1 = FlowbeeLocalData::from_flow(&raw_1, &key_1);
            let mut rtt_1 = RttBuffer::new(
                RttData::from_nanos(1_000_000),
                FlowbeeEffectiveDirection::Download,
                raw_1.last_seen,
            );
            rtt_1.push(
                RttData::from_nanos(1_000_000),
                FlowbeeEffectiveDirection::Download,
                raw_1.last_seen,
            );
            local_1.set_rtt_buffer(rtt_1);
            let mut local_2 = FlowbeeLocalData::from_flow(&raw_2, &key_2);
            let mut rtt_2 = RttBuffer::new(
                RttData::from_nanos(100_000_000),
                FlowbeeEffectiveDirection::Download,
                raw_2.last_seen,
            );
            rtt_2.push(
                RttData::from_nanos(100_000_000),
                FlowbeeEffectiveDirection::Download,
                raw_2.last_seen,
            );
            local_2.set_rtt_buffer(rtt_2);
            flows.insert(
                key_1,
                (local_1, FlowAnalysis::new(&key_1)),
            );
            flows.insert(
                key_2,
                (local_2, FlowAnalysis::new(&key_2)),
            );
        });
        let BusResponse::CountActiveFlows(count) = super::count_active_flows() else {
            panic!("active flow count response should match request");
        };
        assert_eq!(count, 2);
        refresh_active_flow_snapshot();

        let BusResponse::CountActiveFlows(count) = super::count_active_flows() else {
            panic!("active flow count response should match request");
        };
        assert_eq!(count, 2);

        let BusResponse::AllActiveFlows(all_flows) = super::dump_active_flows() else {
            panic!("active flow dump response should match request");
        };
        let first = all_flows
            .iter()
            .find(|flow| flow.local_ip == "192.0.2.10")
            .expect("first flow should be in active-flow dump");
        assert_eq!(first.remote_ip, "198.51.100.20");
        assert_eq!(first.src_port, 443);
        assert_eq!(first.dst_port, 50_000);
        assert_eq!(first.bytes_sent, raw_1.bytes_sent);
        assert_eq!(first.packets_sent, raw_1.packets_sent);
        assert_eq!(first.rate_estimate_bps, raw_1.rate_estimate_bps);
        assert_eq!(first.tcp_retransmits, raw_1.tcp_retransmits);
        assert_eq!(first.tos, raw_1.tos);
        assert_eq!(first.flags, raw_1.flags);
        assert_eq!(first.remote_asn, 0);
        assert_eq!(first.remote_asn_name, "");
        assert_eq!(first.remote_asn_country, "");
        assert_eq!(first.analysis, "HTTPS");
        assert_eq!(first.last_seen, raw_1.last_seen);
        assert_eq!(first.start_time, raw_1.start_time);
        assert_eq!(first.circuit_id, "");
        assert_eq!(first.circuit_name, "");

        let BusResponse::TopFlows(top_flows) = super::top_flows(1, TopFlowType::Bytes) else {
            panic!("top flows response should match request");
        };
        assert_eq!(top_flows.len(), 1);
        assert_eq!(top_flows[0].local_ip, "192.0.2.11");
        assert_eq!(top_flows[0].bytes_sent, raw_2.bytes_sent);

        let BusResponse::TopFlows(top_flows) = super::top_flows(1, TopFlowType::RateEstimate)
        else {
            panic!("top flows response should match request");
        };
        assert_eq!(top_flows.len(), 1);
        assert_eq!(top_flows[0].local_ip, "192.0.2.11");
        assert_eq!(top_flows[0].rate_estimate_bps, raw_2.rate_estimate_bps);

        let BusResponse::TopFlows(top_flows) = super::top_flows(1, TopFlowType::Packets) else {
            panic!("top flows response should match request");
        };
        assert_eq!(top_flows.len(), 1);
        assert_eq!(top_flows[0].local_ip, "192.0.2.11");
        assert_eq!(top_flows[0].packets_sent, raw_2.packets_sent);

        let BusResponse::TopFlows(top_flows) = super::top_flows(1, TopFlowType::Drops) else {
            panic!("top flows response should match request");
        };
        assert_eq!(top_flows.len(), 1);
        assert_eq!(top_flows[0].local_ip, "192.0.2.11");
        assert_eq!(top_flows[0].tcp_retransmits, raw_2.tcp_retransmits);

        let BusResponse::TopFlows(top_flows) = super::top_flows(1, TopFlowType::RoundTripTime)
        else {
            panic!("top flows response should match request");
        };
        assert_eq!(top_flows.len(), 1);
        assert_eq!(top_flows[0].local_ip, "192.0.2.10");
        assert!(top_flows[0].rtt_nanos.down > 0);

        let BusResponse::TopFlows(top_flows) = super::top_flows(0, TopFlowType::Bytes) else {
            panic!("top flows response should match request");
        };
        assert!(top_flows.is_empty());

        let BusResponse::TopFlows(top_flows) = super::top_flows(10, TopFlowType::Bytes) else {
            panic!("top flows response should match request");
        };
        assert_eq!(
            top_flows
                .iter()
                .map(|flow| flow.local_ip.as_str())
                .collect::<Vec<_>>(),
            vec!["192.0.2.11", "192.0.2.10"]
        );

        let BusResponse::FlowsByIp(flows_by_ip) = super::flows_by_ip("192.0.2.10") else {
            panic!("flows-by-ip response should match request");
        };
        assert_eq!(flows_by_ip.len(), 1);
        assert_eq!(flows_by_ip[0].local_ip, "192.0.2.10");
        assert_eq!(flows_by_ip[0].remote_ip, "198.51.100.20");

        replace_active_flows_live_for_test(Vec::new());
        let BusResponse::CountActiveFlows(count) = super::count_active_flows() else {
            panic!("active flow count response should match request");
        };
        assert_eq!(count, 0);
        let BusResponse::FlowsByIp(flows_by_ip) = super::flows_by_ip("192.0.2.10") else {
            panic!("flows-by-ip response should match request");
        };
        assert_eq!(flows_by_ip.len(), 1);

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
    }

    #[test]
    fn active_flow_snapshot_publishes_catalog_metadata_for_bus_readers() {
        let mut ctx = ActiveFlowSnapshotTestContext::with_shaped_devices(
            "active-flow-test",
            vec![ShapedDevice {
                circuit_id: "circuit-meta".to_string(),
                circuit_name: "Circuit Metadata".to_string(),
                device_id: "device-meta".to_string(),
                device_name: "Device Metadata".to_string(),
                parent_node: "Parent".to_string(),
                ipv4: vec![(Ipv4Addr::new(192, 0, 2, 20), 32)],
                ..Default::default()
            }],
        );
        let local_ip = XdpIpAddress::from_ip("192.0.2.20".parse().expect("test IP should parse"));
        let remote_ip =
            XdpIpAddress::from_ip("198.51.100.30".parse().expect("test IP should parse"));

        let mut key = FlowbeeKey::default();
        key.local_ip = local_ip;
        key.remote_ip = remote_ip;
        key.ip_protocol = 6;
        key.src_port = 443;
        key.dst_port = 50_100;
        let mut raw = FlowbeeData::default();
        raw.start_time = 10;
        raw.last_seen = 20;
        raw.bytes_sent = DownUpOrder::new(1_000, 2_000);
        raw.packets_sent = DownUpOrder::new(10, 20);
        raw.rate_estimate_bps = DownUpOrder::new(30_000, 40_000);
        let mut local = FlowbeeLocalData::from_flow(&raw, &key);
        local.set_circuit_id_hint(Some("stale-circuit"));

        mutate_all_flows(|flows| {
            flows.clear();
            flows.insert(key, (local, FlowAnalysis::new(&key)));
        });
        refresh_active_flow_snapshot();

        let BusResponse::FlowsByIp(flows_by_ip) = super::flows_by_ip("192.0.2.20") else {
            panic!("flows-by-ip response should match request");
        };
        assert_eq!(flows_by_ip.len(), 1);
        assert_eq!(flows_by_ip[0].circuit_id, "stale-circuit");
        assert_eq!(flows_by_ip[0].circuit_name, "Circuit Metadata");
        let snapshot = active_flow_snapshot();
        assert_eq!(snapshot[0].circuit_hash, Some(hash_to_i64("circuit-meta")));
        assert_eq!(snapshot[0].device_hash, Some(hash_to_i64("device-meta")));
        assert_eq!(snapshot[0].device_name, "Device Metadata");

        ctx.replace_shaped_devices(
            "active-flow-test-updated",
            vec![ShapedDevice {
                circuit_id: "circuit-meta-updated".to_string(),
                circuit_name: "Circuit Metadata Updated".to_string(),
                device_id: "device-meta-updated".to_string(),
                device_name: "Device Metadata Updated".to_string(),
                parent_node: "Parent".to_string(),
                ipv4: vec![(Ipv4Addr::new(192, 0, 2, 20), 32)],
                ..Default::default()
            }],
        );

        let BusResponse::FlowsByIp(flows_by_ip) = super::flows_by_ip("192.0.2.20") else {
            panic!("flows-by-ip response should match request");
        };
        assert_eq!(flows_by_ip[0].circuit_id, "stale-circuit");

        refresh_active_flow_snapshot();
        let BusResponse::FlowsByIp(flows_by_ip) = super::flows_by_ip("192.0.2.20") else {
            panic!("flows-by-ip response should match request");
        };
        assert_eq!(flows_by_ip[0].circuit_id, "stale-circuit");
        assert_eq!(flows_by_ip[0].circuit_name, "Circuit Metadata Updated");
        assert_eq!(active_flow_snapshot()[0].device_name, "Device Metadata Updated");

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
    }

    #[test]
    fn active_flow_snapshot_prefers_hash_lookup_over_ip_fallback() {
        let ip_matched_device = ShapedDevice {
            circuit_id: "ip-circuit".to_string(),
            circuit_name: "IP Circuit".to_string(),
            device_id: "ip-device".to_string(),
            device_name: "IP Device".to_string(),
            parent_node: "Parent".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 0, 2, 30), 32)],
            ..Default::default()
        };
        let hash_matched_device = ShapedDevice {
            circuit_id: "hash-circuit".to_string(),
            circuit_name: "Hash Circuit".to_string(),
            device_id: "hash-device".to_string(),
            device_name: "Hash Device".to_string(),
            parent_node: "Parent".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 0, 2, 31), 32)],
            ..Default::default()
        };
        let _ctx = ActiveFlowSnapshotTestContext::with_shaped_devices(
            "active-flow-hash-precedence-test",
            vec![ip_matched_device, hash_matched_device],
        );

        let local_ip = XdpIpAddress::from_ip("192.0.2.30".parse().expect("test IP should parse"));
        let catalog = lqos_network_devices::network_devices_catalog();
        let resolved = resolve_flow_device(
            &catalog,
            &local_ip,
            Some(hash_to_i64("hash-device")),
            Some(hash_to_i64("hash-circuit")),
        )
        .expect("hash-backed device should resolve");
        assert_eq!(resolved.device_id, "hash-device");

        let mut key = FlowbeeKey::default();
        key.local_ip = local_ip;
        key.remote_ip =
            XdpIpAddress::from_ip("198.51.100.40".parse().expect("test IP should parse"));
        key.ip_protocol = 6;
        key.src_port = 443;
        key.dst_port = 50_110;
        let mut raw = FlowbeeData::default();
        raw.start_time = 10;
        raw.last_seen = 20;
        raw.bytes_sent = DownUpOrder::new(1_000, 2_000);
        raw.packets_sent = DownUpOrder::new(10, 20);
        raw.rate_estimate_bps = DownUpOrder::new(30_000, 40_000);
        raw.device_hash = hash_to_i64("hash-device") as u64;
        raw.circuit_hash = hash_to_i64("hash-circuit") as u64;

        mutate_all_flows(|flows| {
            flows.clear();
            flows.insert(
                key,
                (
                    FlowbeeLocalData::from_flow(&raw, &key),
                    FlowAnalysis::new(&key),
                ),
            );
        });
        refresh_active_flow_snapshot();

        let snapshot = active_flow_snapshot();
        assert_eq!(snapshot[0].circuit_id, "hash-circuit");
        assert_eq!(snapshot[0].circuit_name, "Hash Circuit");
        assert_eq!(snapshot[0].device_name, "Hash Device");

        mutate_all_flows(|flows| flows.clear());
        refresh_active_flow_snapshot();
    }

    #[test]
    fn insight_topology_conversion_preserves_coordinates() {
        let current = RawNetJsBody {
            download_bandwidth_mbps: 900,
            upload_bandwidth_mbps: 800,
            latitude: Some(31.861_029),
            longitude: Some(-106.549_46),
            site_type: Some("Site".to_string()),
            children: None,
        }
        .to_lts2("Site A");

        let encoded = to_value(&current).expect("topology payload serializes");
        assert_eq!(
            encoded.get("latitude").and_then(|value| value.as_f64()),
            Some(31.861_028_671_264_65_f64)
        );
        assert_eq!(
            encoded.get("longitude").and_then(|value| value.as_f64()),
            Some(-106.549_461_364_746_1_f64)
        );
    }

    #[test]
    fn old_receivers_ignore_topology_coordinate_fields() {
        let current = RawNetJsBody {
            download_bandwidth_mbps: 900,
            upload_bandwidth_mbps: 800,
            latitude: Some(31.861_029),
            longitude: Some(-106.549_46),
            site_type: Some("Site".to_string()),
            children: None,
        }
        .to_lts2("Site A");

        let bytes = serde_cbor::to_vec(&current).expect("current topology payload serializes");
        let decoded: OldLts2NetJs =
            serde_cbor::from_slice(&bytes).expect("legacy topology shape ignores extra fields");

        assert_eq!(decoded.name, "Site A");
        assert_eq!(decoded.download_bandwidth_mbps, 900);
        assert_eq!(decoded.upload_bandwidth_mbps, 800);
        assert_eq!(decoded.site_type.as_deref(), Some("Site"));
    }

    #[test]
    fn circuit_metadata_falls_back_to_ip_when_entry_circuit_id_is_blank() {
        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(vec![ShapedDevice {
            circuit_id: "circuit-1".to_string(),
            circuit_name: "Circuit Alpha".to_string(),
            device_id: "device-1".to_string(),
            parent_node: "Parent-A".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 168, 1, 10), 32)],
            ..Default::default()
        }]);
        let shaped_catalog = ShapedDevicesCatalog::from_shaped_devices(Arc::new(shaped));
        let catalog = NetworkDevicesCatalog::from_snapshots(shaped_catalog, Arc::new(Vec::new()));
        let ip = XdpIpAddress::from_ip("192.168.1.10".parse().expect("test IP should parse"));
        let mut entry = ThroughputEntry {
            circuit_id: None,
            circuit_hash: None,
            device_hash: None,
            network_json_parents: None,
            first_cycle: 0,
            most_recent_cycle: 0,
            bytes: DownUpOrder::zeroed(),
            actual_bytes: DownUpOrder::zeroed(),
            packets: DownUpOrder::zeroed(),
            tcp_packets: DownUpOrder::zeroed(),
            udp_packets: DownUpOrder::zeroed(),
            icmp_packets: DownUpOrder::zeroed(),
            prev_bytes: DownUpOrder::zeroed(),
            prev_actual_bytes: DownUpOrder::zeroed(),
            prev_packets: DownUpOrder::zeroed(),
            prev_tcp_packets: DownUpOrder::zeroed(),
            prev_udp_packets: DownUpOrder::zeroed(),
            prev_icmp_packets: DownUpOrder::zeroed(),
            bytes_per_second: DownUpOrder::zeroed(),
            actual_bytes_per_second: DownUpOrder::zeroed(),
            packets_per_second: DownUpOrder::zeroed(),
            tc_handle: TcHandle::from_u32(0),
            rtt_buffer: Default::default(),
            recent_rtt_data: [RttData::from_nanos(0); 60],
            last_fresh_rtt_data_cycle: 0,
            last_seen: 0,
            tcp_retransmits: DownUpOrder::zeroed(),
            tcp_retransmit_packets: DownUpOrder::zeroed(),
            qoq: QoqScores::default(),
        };

        let (circuit_id, circuit_name) = resolve_circuit_metadata_for_entry(&catalog, &ip, &entry);

        assert_eq!(circuit_id, "circuit-1");
        assert_eq!(circuit_name, "Circuit Alpha");

        entry.circuit_id = Some("hint-circuit".to_string());
        let (circuit_id, circuit_name) = resolve_circuit_metadata_for_entry(&catalog, &ip, &entry);

        assert_eq!(circuit_id, "hint-circuit");
        assert_eq!(circuit_name, "Circuit Alpha");
    }

    #[test]
    fn circuit_current_rtt_p50_nanos_reads_from_shared_circuit_buffer() {
        let old_rtt = CIRCUIT_RTT_BUFFERS.load_full();

        let mut rtt = RttBuffer::default();
        rtt.push(
            RttData::from_nanos(11_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        rtt.push(
            RttData::from_nanos(11_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Upload,
            1,
        );
        rtt.push(
            RttData::from_nanos(31_000_000),
            FlowbeeEffectiveDirection::Upload,
            1,
        );

        let mut rtt_map = FxHashMap::default();
        rtt_map.insert(123_i64, rtt);
        CIRCUIT_RTT_BUFFERS.store(Arc::new(rtt_map));

        let result = circuit_current_rtt_p50_nanos(123);

        assert_eq!(result.down, Some(12_000_000));
        assert_eq!(result.up, Some(35_000_000));

        CIRCUIT_RTT_BUFFERS.store(old_rtt);
    }

    #[test]
    fn circuit_current_qoo_reads_from_shared_circuit_heatmap() {
        let mut heatmap = TemporalQoqHeatmap::new();
        heatmap.add_sample(Some(88.0), Some(77.0));

        let mut qoq_heatmaps = crate::throughput_tracker::THROUGHPUT_TRACKER
            .circuit_qoq_heatmaps
            .lock();
        let old_qoq = qoq_heatmaps.clone();
        qoq_heatmaps.clear();
        qoq_heatmaps.insert(456_i64, heatmap);
        drop(qoq_heatmaps);

        let result = circuit_current_qoo(456);
        assert_eq!(result.down, Some(88.0));
        assert_eq!(result.up, Some(77.0));

        let mut qoq_heatmaps = crate::throughput_tracker::THROUGHPUT_TRACKER
            .circuit_qoq_heatmaps
            .lock();
        *qoq_heatmaps = old_qoq;
    }
}
