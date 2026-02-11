pub mod flow_data;
mod stats_submission;
mod throughput_entry;
mod tracking_data;

use self::flow_data::{
    ALL_FLOWS, FlowAnalysis, FlowbeeLocalData, get_asn_name_and_country, get_asn_name_by_id,
    snapshot_asn_heatmaps,
};
use crate::system_stats::SystemStats;
use crate::throughput_tracker::flow_data::FlowbeeEffectiveDirection;
use crate::{
    lts2_sys::{get_lts_license_status, shared_types::LtsStatus},
    shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICE_HASH_CACHE, SHAPED_DEVICES},
    stats::TIME_TO_POLL_HOSTS,
    throughput_tracker::tracking_data::ThroughputTracker,
};
use arc_swap::ArcSwap;
pub(crate) use flow_data::RttBuffer;
use fxhash::{FxHashMap, FxHashSet};
use lqos_bakery::BakeryCommands;
use lqos_bus::{
    AsnHeatmapData, BusResponse, CircuitHeatmapData, ExecutiveSummaryHeader, FlowbeeProtocol,
    IpStats, SiteHeatmapData, TcHandle, TopFlowType, XdpPpingResult,
};
use lqos_queue_tracker::ALL_QUEUE_SUMMARY;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::units::{DownUpOrder, down_up_divide};
use lqos_utils::{XdpIpAddress, hash_to_i64, unix_time::time_since_boot};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::sync::Arc;
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use tokio::time::{Duration, Instant};
use tracing::{debug, info, warn};

const RETIRE_AFTER_SECONDS: u64 = 30;

pub static THROUGHPUT_TRACKER: Lazy<ThroughputTracker> = Lazy::new(ThroughputTracker::new);
pub(crate) static CIRCUIT_RTT_BUFFERS: Lazy<ArcSwap<FxHashMap<i64, RttBuffer>>> =
    Lazy::new(|| ArcSwap::new(Arc::new(FxHashMap::default())));

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

    // Obtain the netflow_enabled from the config, default to false
    let netflow_enabled = if let Ok(config) = lqos_config::load_config() {
        if let Some(flow_config) = &config.flows {
            flow_config.netflow_enabled
        } else {
            false
        }
    } else {
        false
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
    let mut expired_flows: Vec<FlowbeeKey> = Vec::new();

    // Counter for occasional stats
    let mut stats_counter = 0;

    loop {
        let start = Instant::now();
        timer_metrics.zero();

        // Formerly a "spawn blocking" blob
        {
            let mut net_json_calc = NETWORK_JSON.write();
            timer_metrics.update_cycle = timer_metrics.start.elapsed().as_secs_f64();
            net_json_calc.zero_throughput_and_rtt();
            timer_metrics.zero_throughput_and_rtt = timer_metrics.start.elapsed().as_secs_f64();
            THROUGHPUT_TRACKER.copy_previous_and_reset_rtt();
            timer_metrics.copy_previous_and_reset_rtt = timer_metrics.start.elapsed().as_secs_f64();
            THROUGHPUT_TRACKER
                .apply_new_throughput_counters(&mut net_json_calc, bakery_sender.clone());
            timer_metrics.apply_new_throughput_counters =
                timer_metrics.start.elapsed().as_secs_f64();
            THROUGHPUT_TRACKER.apply_flow_data(
                timeout_seconds,
                netflow_enabled,
                netflow_sender.clone(),
                &mut net_json_calc,
                &mut rtt_circuit_tracker,
                &mut rtt_by_circuit,
                &mut tcp_retries,
                &mut expired_flows,
            );
            CIRCUIT_RTT_BUFFERS.store(Arc::new(rtt_by_circuit.clone()));
            THROUGHPUT_TRACKER.record_circuit_heatmaps();
            let enable_site_heatmaps = lqos_config::load_config()
                .map(|config| config.enable_site_heatmaps)
                .unwrap_or(true);
            net_json_calc.record_site_heatmaps(enable_site_heatmaps);

            // Clean up work tables
            rtt_circuit_tracker.clear();
            rtt_by_circuit.clear();
            tcp_retries.clear();
            expired_flows.clear();
            rtt_circuit_tracker.shrink_to_fit();
            rtt_by_circuit.shrink_to_fit();
            tcp_retries.shrink_to_fit();
            expired_flows.shrink_to_fit();

            timer_metrics.apply_flow_data = timer_metrics.start.elapsed().as_secs_f64();
            THROUGHPUT_TRACKER.apply_queue_stats(&mut net_json_calc);
            timer_metrics.apply_queue_stats = timer_metrics.start.elapsed().as_secs_f64();
            THROUGHPUT_TRACKER.update_totals();
            timer_metrics.update_totals = timer_metrics.start.elapsed().as_secs_f64();
            THROUGHPUT_TRACKER.next_cycle();
            timer_metrics.next_cycle = timer_metrics.start.elapsed().as_secs_f64();
            std::mem::drop(net_json_calc);
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
        } else {
            if let Some(last) = last_submitted_to_lts {
                let elapsed_f64 = last.elapsed().as_secs_f64();
                // Temporary: place this in a thread to not block the timer
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
            THROUGHPUT_TRACKER.bits_per_second(),
            THROUGHPUT_TRACKER.packets_per_second(),
            THROUGHPUT_TRACKER.shaped_bits_per_second(),
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
fn retire_check(cycle: u64, recent_cycle: u64) -> bool {
    cycle < recent_cycle + RETIRE_AFTER_SECONDS
}

type TopList = (
    XdpIpAddress,
    DownUpOrder<u64>,
    DownUpOrder<u64>,
    f32,
    TcHandle,
    String,
    (f64, f64),
);

pub fn top_n(start: u32, end: u32) -> BusResponse {
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .map(|(k, te)| {
                (
                    *k,
                    te.bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
                    down_up_divide(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    full_list.sort_by(|a, b| b.1.down.cmp(&a.1.down));
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
                tcp_retransmits: *tcp_retransmits,
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
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .map(|(k, te)| {
                (
                    *k,
                    te.bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
                    down_up_divide(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    full_list.sort_by(|a, b| b.1.up.cmp(&a.1.up));
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
                tcp_retransmits: *tcp_retransmits,
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

    let devices = SHAPED_DEVICES.load();
    let mut circuit_meta: FxHashMap<i64, (String, String)> = FxHashMap::default();
    devices.devices.iter().for_each(|device| {
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

    let reader = NETWORK_JSON.read();
    let mut rows: Vec<SiteHeatmapData> = reader
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
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(_k, te)| te.median_latency().is_some())
            .map(|(k, te)| {
                (
                    *k,
                    te.bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
                    down_up_divide(te.tcp_retransmits, te.tcp_packets),
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
                tcp_retransmits: *tcp_retransmits,
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
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(_k, te)| te.median_latency().is_some())
            .map(|(k, te)| {
                (
                    *k,
                    te.bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
                    down_up_divide(te.tcp_retransmits, te.tcp_packets),
                )
            })
            .collect()
    };
    // Use a total order for floating-point comparison to avoid panics
    // when NaN/Inf are present and ensure comparator transitivity.
    full_list.sort_by(|a, b| {
        let total_a = a.6.0 + a.6.1;
        let total_b = b.6.0 + b.6.1;
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
                tcp_retransmits: *tcp_retransmits,
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
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(k, _v)| !k.as_ip().is_loopback())
            .filter(|(_k, d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(_k, te)| te.median_latency().is_some())
            .map(|(k, te)| {
                (
                    *k,
                    te.bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
                    down_up_divide(te.tcp_retransmits, te.tcp_packets),
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
                tcp_retransmits: *tcp_retransmits,
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
        min: samples[0] as f32,
        max: samples[samples.len() - 1] as f32,
        median: samples[samples.len() / 2] as f32,
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
    let devices = SHAPED_DEVICES.load();
    let circuit_count = devices
        .devices
        .iter()
        .map(|device| device.circuit_hash)
        .collect::<FxHashSet<_>>()
        .len() as u64;
    let device_count = devices.devices.len() as u64;

    let site_count = {
        let reader = NETWORK_JSON.read();
        let total_nodes = reader.get_nodes_when_ready().len();
        // Remove the synthetic root node when counting sites.
        total_nodes.saturating_sub(1) as u64
    };

    let (total_hosts, shaped_hosts) = current_host_counts();
    let mapped_ip_count = shaped_hosts as u64;
    let unmapped_ip_count = total_hosts.saturating_sub(shaped_hosts) as u64;

    let queue_counts = ALL_QUEUE_SUMMARY.queue_counts();
    let insight_connected = !matches!(
        get_lts_license_status().0,
        LtsStatus::Invalid | LtsStatus::NotChecked
    );

    BusResponse::ExecutiveSummaryHeader(ExecutiveSummaryHeader {
        circuit_count,
        device_count,
        site_count,
        mapped_ip_count,
        unmapped_ip_count,
        htb_queue_count: queue_counts.htb as u64,
        cake_queue_count: queue_counts.cake as u64,
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
    full_list.sort_by(|a, b| b.5.cmp(&a.5));
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
                tcp_retransmits: (0.0, 0.0),
            },
        )
        .collect();
    BusResponse::AllUnknownIps(result)
}

/// For debugging: dump all active flows!
pub fn dump_active_flows() -> BusResponse {
    let lock = ALL_FLOWS.lock();
    let result: Vec<lqos_bus::FlowbeeSummaryData> = lock
        .flow_data
        .iter()
        .map(|(key, row)| {
            let geo = get_asn_name_and_country(key.remote_ip.as_ip());

            let (circuit_id, circuit_name) = (String::new(), String::new());

            lqos_bus::FlowbeeSummaryData {
                remote_ip: key.remote_ip.as_ip().to_string(),
                local_ip: key.local_ip.as_ip().to_string(),
                src_port: key.src_port,
                dst_port: key.dst_port,
                ip_protocol: FlowbeeProtocol::from(key.ip_protocol),
                bytes_sent: row.0.bytes_sent,
                packets_sent: row.0.packets_sent,
                rate_estimate_bps: row.0.rate_estimate_bps,
                tcp_retransmits: row.0.tcp_retransmits,
                end_status: row.0.end_status,
                tos: row.0.tos,
                flags: row.0.get_flags(),
                remote_asn: row.1.asn_id.0,
                remote_asn_name: geo.name,
                remote_asn_country: geo.country,
                analysis: row.1.protocol_analysis.to_string(),
                last_seen: row.0.last_seen,
                start_time: row.0.start_time,
                rtt_nanos: DownUpOrder::new(
                    row.0
                        .get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download),
                    row.0
                        .get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Upload),
                ),
                circuit_id,
                circuit_name,
            }
        })
        .collect();

    BusResponse::AllActiveFlows(result)
}

/// Count active flows
pub fn count_active_flows() -> BusResponse {
    let lock = ALL_FLOWS.lock();
    BusResponse::CountActiveFlows(lock.flow_data.len() as u64)
}

/// Top Flows Report
pub fn top_flows(n: u32, flow_type: TopFlowType) -> BusResponse {
    let lock = ALL_FLOWS.lock();
    let mut table: Vec<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))> = lock
        .flow_data
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    std::mem::drop(lock); // Early lock release

    match flow_type {
        TopFlowType::RateEstimate => {
            table.sort_by(|a, b| {
                let a_total = a.1.0.rate_estimate_bps.sum();
                let b_total = b.1.0.rate_estimate_bps.sum();
                b_total.cmp(&a_total)
            });
        }
        TopFlowType::Bytes => {
            table.sort_by(|a, b| {
                let a_total = a.1.0.bytes_sent.sum();
                let b_total = b.1.0.bytes_sent.sum();
                b_total.cmp(&a_total)
            });
        }
        TopFlowType::Packets => {
            table.sort_by(|a, b| {
                let a_total = a.1.0.packets_sent.sum();
                let b_total = b.1.0.packets_sent.sum();
                b_total.cmp(&a_total)
            });
        }
        TopFlowType::Drops => {
            table.sort_by(|a, b| {
                let a_total = a.1.0.tcp_retransmits.sum();
                let b_total = b.1.0.tcp_retransmits.sum();
                b_total.cmp(&a_total)
            });
        }
        TopFlowType::RoundTripTime => {
            table.sort_by(|a, b| {
                let a_total = a.1.0.get_rtt(FlowbeeEffectiveDirection::Download);
                let b_total = b.1.0.get_rtt(FlowbeeEffectiveDirection::Download);
                a_total.cmp(&b_total)
            });
        }
    }

    let shaped = SHAPED_DEVICES.load();
    let shaped_cache = SHAPED_DEVICE_HASH_CACHE.load();
    let throughput = THROUGHPUT_TRACKER.raw_data.lock();

    let result = table
        .iter()
        .take(n as usize)
        .map(|(ip, flow)| {
            let geo = get_asn_name_and_country(ip.remote_ip.as_ip());

            let mut circuit_id = String::new();
            let mut circuit_name = String::new();
            if let Some(te) = throughput.get(&ip.local_ip) {
                if let Some(id) = &te.circuit_id {
                    circuit_id = id.clone();
                }
                let shaped_device = te
                    .device_hash
                    .and_then(|hash| shaped_cache.index_by_device_hash(&shaped, hash))
                    .or_else(|| {
                        te.circuit_hash
                            .and_then(|hash| shaped_cache.index_by_circuit_hash(&shaped, hash))
                    })
                    .and_then(|idx| shaped.devices.get(idx));
                if let Some(device) = shaped_device {
                    if circuit_id.is_empty() {
                        circuit_id = device.circuit_id.clone();
                    }
                    circuit_name = device.circuit_name.clone();
                }
            }

            lqos_bus::FlowbeeSummaryData {
                remote_ip: ip.remote_ip.as_ip().to_string(),
                local_ip: ip.local_ip.as_ip().to_string(),
                src_port: ip.src_port,
                dst_port: ip.dst_port,
                ip_protocol: FlowbeeProtocol::from(ip.ip_protocol),
                bytes_sent: flow.0.bytes_sent,
                packets_sent: flow.0.packets_sent,
                rate_estimate_bps: flow.0.rate_estimate_bps,
                tcp_retransmits: flow.0.tcp_retransmits,
                end_status: flow.0.end_status,
                tos: flow.0.tos,
                flags: flow.0.get_flags(),
                remote_asn: flow.1.asn_id.0,
                remote_asn_name: geo.name,
                remote_asn_country: geo.country,
                analysis: flow.1.protocol_analysis.to_string(),
                last_seen: flow.0.last_seen,
                start_time: flow.0.start_time,
                rtt_nanos: DownUpOrder::new(
                    flow.0
                        .get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download),
                    flow.0
                        .get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Upload),
                ),
                circuit_id,
                circuit_name,
            }
        })
        .collect();

    BusResponse::TopFlows(result)
}

/// Flows by IP
pub fn flows_by_ip(ip: &str) -> BusResponse {
    if let Ok(ip) = ip.parse::<IpAddr>() {
        let ip = XdpIpAddress::from_ip(ip);
        let lock = ALL_FLOWS.lock();
        let throughput = THROUGHPUT_TRACKER.raw_data.lock();
        let shaped = SHAPED_DEVICES.load();
        let shaped_cache = SHAPED_DEVICE_HASH_CACHE.load();
        let (circuit_id, circuit_name) = {
            let mut circuit_id = String::new();
            let mut circuit_name = String::new();
            if let Some(te) = throughput.get(&ip) {
                if let Some(id) = &te.circuit_id {
                    circuit_id = id.clone();
                }
                let shaped_device = te
                    .device_hash
                    .and_then(|hash| shaped_cache.index_by_device_hash(&shaped, hash))
                    .or_else(|| {
                        te.circuit_hash
                            .and_then(|hash| shaped_cache.index_by_circuit_hash(&shaped, hash))
                    })
                    .and_then(|idx| shaped.devices.get(idx));
                if let Some(device) = shaped_device {
                    if circuit_id.is_empty() {
                        circuit_id = device.circuit_id.clone();
                    }
                    circuit_name = device.circuit_name.clone();
                }
            }
            (circuit_id, circuit_name)
        };
        let matching_flows: Vec<_> = lock
            .flow_data
            .iter()
            .filter(|(key, _)| key.local_ip == ip)
            .map(|(key, row)| {
                let geo = get_asn_name_and_country(key.remote_ip.as_ip());

                lqos_bus::FlowbeeSummaryData {
                    remote_ip: key.remote_ip.as_ip().to_string(),
                    local_ip: key.local_ip.as_ip().to_string(),
                    src_port: key.src_port,
                    dst_port: key.dst_port,
                    ip_protocol: FlowbeeProtocol::from(key.ip_protocol),
                    bytes_sent: row.0.bytes_sent,
                    packets_sent: row.0.packets_sent,
                    rate_estimate_bps: row.0.rate_estimate_bps,
                    tcp_retransmits: row.0.tcp_retransmits,
                    end_status: row.0.end_status,
                    tos: row.0.tos,
                    flags: row.0.get_flags(),
                    remote_asn: row.1.asn_id.0,
                    remote_asn_name: geo.name,
                    remote_asn_country: geo.country,
                    analysis: row.1.protocol_analysis.to_string(),
                    last_seen: row.0.last_seen,
                    start_time: row.0.start_time,
                    rtt_nanos: DownUpOrder::new(
                        row.0
                            .get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Download),
                        row.0
                            .get_summary_rtt_as_nanos(FlowbeeEffectiveDirection::Upload),
                    ),
                    circuit_id: circuit_id.clone(),
                    circuit_name: circuit_name.clone(),
                }
            })
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
#[derive(Debug, Clone, Serialize)]
pub struct Lts2Circuit {
    pub circuit_id: String,
    pub circuit_name: String,
    pub circuit_hash: i64,
    pub download_min_mbps: u32,
    pub upload_min_mbps: u32,
    pub download_max_mbps: u32,
    pub upload_max_mbps: u32,
    pub parent_node: i64,
    pub parent_node_name: Option<String>,
    pub devices: Vec<Lts2Device>,
}

#[repr(C)]
#[derive(Debug, Clone, Serialize)]
pub struct Lts2Device {
    pub device_id: String,
    pub device_name: String,
    pub device_hash: i64,
    pub mac: String,
    pub ipv4: Vec<([u8; 4], u8)>,
    pub ipv6: Vec<([u8; 16], u8)>,
    pub comment: String,
}
