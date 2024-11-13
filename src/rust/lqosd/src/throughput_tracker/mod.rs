pub mod flow_data;
mod throughput_entry;
mod tracking_data;
use std::fs::read_to_string;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use fxhash::FxHashMap;
use self::flow_data::{get_asn_name_and_country, FlowAnalysis, FlowbeeLocalData, ALL_FLOWS};
use crate::{
    long_term_stats::get_network_tree,
    shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES, STATS_NEEDS_NEW_SHAPED_DEVICES},
    stats::TIME_TO_POLL_HOSTS,
    throughput_tracker::tracking_data::ThroughputTracker,
};
use tracing::{debug, warn};
use lqos_bus::{BusResponse, FlowbeeProtocol, IpStats, TcHandle, TopFlowType, XdpPpingResult};
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::{hash_to_i64, unix_time::time_since_boot, XdpIpAddress};
use lts_client::collector::{HostSummary, stats_availability::StatsUpdateMessage, ThroughputSummary};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use tokio::{
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use lqos_config::load_config;
use lqos_queue_tracker::{ALL_QUEUE_SUMMARY, TOTAL_QUEUE_STATS};
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::unix_now;
use lts2_sys::shared_types::{CircuitCakeDrops, CircuitCakeMarks};
use crate::system_stats::SystemStats;
use crate::throughput_tracker::flow_data::RttData;

const RETIRE_AFTER_SECONDS: u64 = 30;

pub static THROUGHPUT_TRACKER: Lazy<ThroughputTracker> = Lazy::new(ThroughputTracker::new);

/// Create the throughput monitor thread, and begin polling for
/// throughput data every second.
///
/// ## Arguments
///
/// * `long_term_stats_tx` - an optional MPSC sender to notify the
///   collection thread that there is fresh data.
pub fn spawn_throughput_monitor(
    long_term_stats_tx: Sender<StatsUpdateMessage>,
    netflow_sender: crossbeam_channel::Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
    system_usage_actor: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) -> anyhow::Result<()> {
    debug!("Starting the bandwidth monitor thread.");
    std::thread::Builder::new()
        .name("Throughput Monitor".to_string())
    .spawn(|| {throughput_task(
        long_term_stats_tx,
        netflow_sender,
        system_usage_actor,
    )})?;

    Ok(())
}

/// Used for tracking the "tick" time, with a view to
/// finding where some code is stalling.
#[derive(Debug)]
struct ThroughputTaskTimeMetrics {
    start: Instant,
    update_cycle : f64,
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
    long_term_stats_tx: Sender<StatsUpdateMessage>,
    netflow_sender: crossbeam_channel::Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
    system_usage_actor: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
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
    let mut tfd = TimerFd::new().unwrap();
    assert_eq!(tfd.get_state(), TimerState::Disarmed);
    tfd.set_state(TimerState::Periodic{
        current: Duration::new(1, 0),
        interval: Duration::new(1, 0)}
                  , SetTimeFlags::Default
    );
    let mut timer_metrics = ThroughputTaskTimeMetrics::new();

    // Preallocate some buffers to avoid allocations in the loop
    let mut rtt_circuit_tracker: FxHashMap<XdpIpAddress, [Vec<RttData>; 2]> = FxHashMap::default();
    let mut tcp_retries: FxHashMap<XdpIpAddress, DownUpOrder<u64>> = FxHashMap::default();
    let mut expired_flows: Vec<FlowbeeKey> = Vec::new();

    // Counter for occasional stats
    let mut stats_counter = 0;

    loop {
        let start = Instant::now();
        timer_metrics.zero();

        // Formerly a "spawn blocking" blob
        {
            let mut net_json_calc = NETWORK_JSON.write().unwrap();
            timer_metrics.update_cycle = timer_metrics.start.elapsed().as_secs_f64();
            net_json_calc.zero_throughput_and_rtt();
            timer_metrics.zero_throughput_and_rtt = timer_metrics.start.elapsed().as_secs_f64();
            THROUGHPUT_TRACKER.copy_previous_and_reset_rtt();
            timer_metrics.copy_previous_and_reset_rtt = timer_metrics.start.elapsed().as_secs_f64();
            THROUGHPUT_TRACKER.apply_new_throughput_counters(&mut net_json_calc);
            timer_metrics.apply_new_throughput_counters = timer_metrics.start.elapsed().as_secs_f64();
            THROUGHPUT_TRACKER.apply_flow_data(
                timeout_seconds,
                netflow_enabled,
                netflow_sender.clone(),
                &mut net_json_calc,
                &mut rtt_circuit_tracker,
                &mut tcp_retries,
                &mut expired_flows,
            );

            // Clean up work tables
            rtt_circuit_tracker.clear();
            tcp_retries.clear();
            expired_flows.clear();
            rtt_circuit_tracker.shrink_to_fit();
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
            submit_throughput_stats(long_term_stats_tx.clone(), 1.0, stats_counter, system_usage_actor.clone());
        } else {
            let elapsed = last_submitted_to_lts.unwrap().elapsed();
            let elapsed_f64 = elapsed.as_secs_f64();
            // Temporary: place this in a thread to not block the timer
            let my_lts_tx = long_term_stats_tx.clone();
            let my_system_usage_actor = system_usage_actor.clone();
            std::thread::Builder::new().name("Throughput Stats Submit".to_string()).spawn(move || {
                submit_throughput_stats(my_lts_tx, elapsed_f64, stats_counter, my_system_usage_actor);
            }).unwrap().join().unwrap();
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

fn scale_u64_by_f64(value: u64, scale: f64) -> u64 {
    (value as f64 * scale) as u64
}

#[derive(Debug)]
struct LtsSubmitMetrics {
    start: Instant,
    shaped_devices: f64,
    total_throughput: f64,
    hosts: f64,
    summary: f64,
    send: f64,
}

impl LtsSubmitMetrics {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            shaped_devices: 0.0,
            total_throughput: 0.0,
            hosts: 0.0,
            summary: 0.0,
            send: 0.0,
        }
    }
}

fn submit_throughput_stats(
    long_term_stats_tx: Sender<StatsUpdateMessage>,
    scale: f64,
    counter: u8,
    system_usage_actor: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) {
    let mut metrics = LtsSubmitMetrics::new();
    let mut lts2_needs_shaped_devices = false;
    // If ShapedDevices has changed, notify the stats thread
    if let Ok(changed) = STATS_NEEDS_NEW_SHAPED_DEVICES.compare_exchange(
        true,
        false,
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
    ) {
        if changed {
            let shaped_devices = SHAPED_DEVICES.load().devices.clone();
            let _ = long_term_stats_tx
                .blocking_send(StatsUpdateMessage::ShapedDevicesChanged(shaped_devices));
            lts2_needs_shaped_devices = true;
        }
    }
    metrics.shaped_devices = metrics.start.elapsed().as_secs_f64();

    // Gather Global Stats
    let packets_per_second = (
        THROUGHPUT_TRACKER
            .packets_per_second.get_down(),
        THROUGHPUT_TRACKER
            .packets_per_second.get_up(),
    );
    let tcp_packets_per_second = (
        THROUGHPUT_TRACKER
            .tcp_packets_per_second.get_down(),
        THROUGHPUT_TRACKER
            .tcp_packets_per_second.get_up(),
    );
    let udp_packets_per_second = (
        THROUGHPUT_TRACKER
            .udp_packets_per_second.get_down(),
        THROUGHPUT_TRACKER
            .udp_packets_per_second.get_up(),
    );
    let icmp_packets_per_second = (
        THROUGHPUT_TRACKER
            .icmp_packets_per_second.get_down(),
        THROUGHPUT_TRACKER
            .icmp_packets_per_second.get_up(),
    );
    let bits_per_second = THROUGHPUT_TRACKER.bits_per_second();
    let shaped_bits_per_second = THROUGHPUT_TRACKER.shaped_bits_per_second();
    metrics.total_throughput = metrics.start.elapsed().as_secs_f64();
    
    if let Ok(config) = load_config() {
        if bits_per_second.down > (config.queues.downlink_bandwidth_mbps as u64 * 1_000_000) {
            debug!("Spike detected - not submitting LTS");
            return; // Do not submit these stats
        }
        if bits_per_second.up > (config.queues.uplink_bandwidth_mbps as u64 * 1_000_000) {
            debug!("Spike detected - not submitting LTS");
            return; // Do not submit these stats
        }
    }
    
    let hosts = THROUGHPUT_TRACKER
        .raw_data
        .lock().unwrap()
        .iter()
        //.filter(|host| host.median_latency().is_some())
        .map(|(k,host)| HostSummary {
            ip: k.as_ip(),
            circuit_id: host.circuit_id.clone(),
            bits_per_second: (scale_u64_by_f64(host.bytes_per_second.down * 8, scale), scale_u64_by_f64(host.bytes_per_second.up * 8, scale)),
            median_rtt: host.median_latency().unwrap_or(0.0),
        })
        .collect();
    metrics.hosts = metrics.start.elapsed().as_secs_f64();

    let summary = Box::new((
        ThroughputSummary {
            bits_per_second: (scale_u64_by_f64(bits_per_second.down, scale), scale_u64_by_f64(bits_per_second.up, scale)),
            shaped_bits_per_second: (scale_u64_by_f64(shaped_bits_per_second.down, scale), scale_u64_by_f64(shaped_bits_per_second.up, scale)),
            packets_per_second,
            hosts,
        },
        get_network_tree(),
    ));
    metrics.summary = metrics.start.elapsed().as_secs_f64();

    // Send the stats
    let _ = lts2_sys::update_config();
    let result = long_term_stats_tx
        .blocking_send(StatsUpdateMessage::ThroughputReady(summary));
    if let Err(e) = result {
        warn!("Error sending message to stats collection system. {e:?}");
    }
    metrics.send = metrics.start.elapsed().as_secs_f64();

    if metrics.start.elapsed().as_secs_f64() > 1.0 {
        warn!("{:?}", metrics);
    }

    /////////////////////////////////////////////////////////////////
    // LTS2 Block
    if let Ok(now) = unix_now() {
        // LTS2 Shaped Devices
        if lts2_needs_shaped_devices {
            // Send the topology tree
            {
                if let Ok(config) = load_config() {
                    let filename = Path::new(&config.lqos_directory).join("network.json");
                    if let Ok(raw_string) = read_to_string(filename) {
                        match serde_json::from_str::<RawNetJs>(&raw_string) {
                            Err(e) => {
                                warn!("Unable to parse network.json. {e:?}");
                            }
                            Ok(json) => {
                                let lts2_format: Vec<_> = json.iter().map(|(k,v)| v.to_lts2(&k)).collect();
                                if let Ok(bytes) = serde_cbor::to_vec(&lts2_format) {
                                    if let Err(e) = lts2_sys::network_tree(now, &bytes) {
                                        warn!("Error sending message to LTS2. {e:?}");
                                    }
                                }
                            }
                        }
                    } else {
                        warn!("Unable to read network.json");
                    }
                }
            }

            // Send the shaped devices
            let shaped_devices = SHAPED_DEVICES.load().devices.clone();
            let mut circuit_map: FxHashMap<String, Lts2Circuit> = FxHashMap::default();
            for device in shaped_devices.into_iter() {
                if let Some(circuit) = circuit_map.get_mut(&device.circuit_id) {
                    circuit.devices.push(Lts2Device {
                        device_hash: device.device_hash,
                        device_id: device.device_id,
                        device_name: device.device_name,
                        mac: device.mac,
                        ipv4: device.ipv4.into_iter().map(ip4_to_bytes).collect(),
                        ipv6: device.ipv6.into_iter().map(ip6_to_bytes).collect(),
                        comment: device.comment,
                    })
                } else {
                    let circuit_hash = device.circuit_hash;
                    circuit_map.insert(
                        device.circuit_id.clone(),
                        Lts2Circuit {
                            circuit_id: device.circuit_id,
                            circuit_name: device.circuit_name,
                            circuit_hash,
                            download_min_mbps: device.download_min_mbps,
                            upload_min_mbps: device.upload_min_mbps,
                            download_max_mbps: device.download_max_mbps,
                            upload_max_mbps: device.upload_max_mbps,
                            parent_node: device.parent_hash,
                            devices: vec![Lts2Device {
                                device_hash: device.device_hash,
                                device_id: device.device_id,
                                device_name: device.device_name,
                                mac: device.mac,
                                ipv4: device.ipv4.into_iter().map(ip4_to_bytes).collect(),
                                ipv6: device.ipv6.into_iter().map(ip6_to_bytes).collect(),
                                comment: device.comment,
                            }],
                        }
                    );
                }
            }
            let devices_as_vec: Vec<Lts2Circuit> = circuit_map.into_iter().map(|(_, v)| v).collect();
            // Serialize via cbor
            if let Ok(bytes) = serde_cbor::to_vec(&devices_as_vec) {
                if lts2_sys::shaped_devices(now, &bytes).is_err() {
                    warn!("Error sending message to LTS2.");
                }
            }
        }

        // Send top-level throughput stats to LTS2
        let bytes = THROUGHPUT_TRACKER.bytes_per_second.as_down_up();
        let shaped_bytes = THROUGHPUT_TRACKER.shaped_bytes_per_second.as_down_up();
        let mut min_rtt = None;
        let mut max_rtt = None;
        let mut median_rtt = None;
        if let Some(rtt_data) = min_max_median_rtt() {
            min_rtt = Some(rtt_data.min);
            max_rtt = Some(rtt_data.max);
            median_rtt = Some(rtt_data.median);
        }
        let tcp_retransmits = min_max_median_tcp_retransmits();
        if lts2_sys::total_throughput(now,
                                      scale_u64_by_f64(bytes.down, scale), scale_u64_by_f64(bytes.up, scale), scale_u64_by_f64(shaped_bytes.down, scale), scale_u64_by_f64(shaped_bytes.up, scale),
                                      scale_u64_by_f64(packets_per_second.0, scale), scale_u64_by_f64(packets_per_second.1, scale),
                                      scale_u64_by_f64(tcp_packets_per_second.0, scale), scale_u64_by_f64(tcp_packets_per_second.1, scale),
                                      scale_u64_by_f64(udp_packets_per_second.0, scale), scale_u64_by_f64(udp_packets_per_second.1, scale),
                                      scale_u64_by_f64(icmp_packets_per_second.0, scale), scale_u64_by_f64(icmp_packets_per_second.1, scale),
                                      min_rtt, max_rtt, median_rtt,
                                      tcp_retransmits.down, tcp_retransmits.up,
                                      TOTAL_QUEUE_STATS.marks.get_down() as i32, TOTAL_QUEUE_STATS.marks.get_up() as i32,
                                      TOTAL_QUEUE_STATS.drops.get_down() as i32, TOTAL_QUEUE_STATS.drops.get_up() as i32,
        ).is_err() {
            warn!("Error sending message to LTS2.");
        }

        // Send per-circuit stats to LTS2
        // Start by combining the throughput data for each circuit as a whole
        struct CircuitThroughputTemp {
            bytes: DownUpOrder<u64>,
            packets: DownUpOrder<u64>,
            tcp_packets: DownUpOrder<u64>,
            udp_packets: DownUpOrder<u64>,
            icmp_packets: DownUpOrder<u64>,
        }

        let mut circuit_throughput: FxHashMap<i64, CircuitThroughputTemp> = FxHashMap::default();
        let mut circuit_retransmits: FxHashMap<i64, DownUpOrder<u64>> = FxHashMap::default();
        let mut circuit_rtt: FxHashMap<i64, Vec<f32>> = FxHashMap::default();

        THROUGHPUT_TRACKER
            .raw_data
            .lock().unwrap()
            .iter()
            .filter(|(k,h)| h.circuit_id.is_some() && h.bytes_per_second.not_zero())
            .for_each(|(k,h)| {
                if let Some(c) = circuit_throughput.get_mut(&h.circuit_hash.unwrap()) {
                    c.bytes += h.bytes_per_second;
                    c.packets += h.packets_per_second;
                    c.tcp_packets += h.tcp_packets;
                    c.udp_packets += h.udp_packets;
                    c.icmp_packets += h.icmp_packets;
                } else {
                    circuit_throughput.insert(h.circuit_hash.unwrap(), CircuitThroughputTemp {
                        bytes: h.bytes_per_second,
                        packets: h.packets_per_second,
                        tcp_packets: h.tcp_packets,
                        udp_packets: h.udp_packets,
                        icmp_packets: h.icmp_packets,
                    });
                }
            });

        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .unwrap()
            .iter()
            .filter(|(k,h)| h.circuit_id.is_some() && h.tcp_retransmits.not_zero())
            .for_each(|(k,h)| {
                if let Some(c) = circuit_retransmits.get_mut(&h.circuit_hash.unwrap()) {
                    *c += h.tcp_retransmits;
                } else {
                    circuit_retransmits.insert(h.circuit_hash.unwrap(), h.tcp_retransmits);
                }
            });

        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .unwrap()
            .iter()
            .filter(|(k,h)| h.circuit_id.is_some() && h.median_latency().is_some())
            .for_each(|(k,h)| {
                if let Some(c) = circuit_rtt.get_mut(&h.circuit_hash.unwrap()) {
                    c.push(h.median_latency().unwrap());
                } else {
                    circuit_rtt.insert(h.circuit_hash.unwrap(), vec![h.median_latency().unwrap()]);
                }
            });

        // And now we send it
        let circuit_throughput_batch = circuit_throughput
            .into_iter()
            .map(|(k,v)| {
                lts2_sys::shared_types::CircuitThroughput {
                    timestamp: now,
                    circuit_hash: k,
                    download_bytes: scale_u64_by_f64(v.bytes.down, scale),
                    upload_bytes: scale_u64_by_f64(v.bytes.up, scale),
                    packets_down: scale_u64_by_f64(v.packets.down, scale),
                    packets_up: scale_u64_by_f64(v.packets.up, scale),
                    packets_tcp_down: scale_u64_by_f64(v.tcp_packets.down, scale),
                    packets_tcp_up: scale_u64_by_f64(v.tcp_packets.up, scale),
                    packets_udp_down: scale_u64_by_f64(v.udp_packets.down, scale),
                    packets_udp_up: scale_u64_by_f64(v.udp_packets.up, scale),
                    packets_icmp_down: scale_u64_by_f64(v.icmp_packets.down, scale),
                    packets_icmp_up: scale_u64_by_f64(v.icmp_packets.up, scale),
                }
            })
            .collect::<Vec<_>>();
        if lts2_sys::circuit_throughput(&circuit_throughput_batch).is_err() {
            warn!("Error sending message to LTS2.");
        }

        let circuit_retransmits_batch = circuit_retransmits
            .into_iter()
            .map(|(k,v)| {
                lts2_sys::shared_types::CircuitRetransmits {
                    timestamp: now,
                    circuit_hash: k,
                    tcp_retransmits_down: v.down as i32,
                    tcp_retransmits_up: v.up as i32,
                }
            })
            .collect::<Vec<_>>();
        if lts2_sys::circuit_retransmits(&circuit_retransmits_batch).is_err() {
            warn!("Error sending message to LTS2.");
        }

        let circuit_rtt_batch = circuit_rtt
            .into_iter()
            .map(|(k,v)| {
                lts2_sys::shared_types::CircuitRtt {
                    timestamp: now,
                    circuit_hash: k,
                    median_rtt: v.iter().sum::<f32>() / v.len() as f32,
                }
            })
            .collect::<Vec<_>>();
        if lts2_sys::circuit_rtt(&circuit_rtt_batch).is_err() {
            warn!("Error sending message to LTS2.");
        }

        // Per host CAKE stats
        let mut cake_drops: Vec<CircuitCakeDrops> = Vec::new();
        let mut cake_marks: Vec<CircuitCakeMarks> = Vec::new();
        ALL_QUEUE_SUMMARY.iterate_queues(|circuit_hash, drops, marks| {
            if drops.not_zero() {
                cake_drops.push(CircuitCakeDrops {
                    timestamp: now,
                    circuit_hash,
                    cake_drops_down: drops.get_down() as i32,
                    cake_drops_up: drops.get_up() as i32,
                });
            }
            if marks.not_zero() {
                cake_marks.push(CircuitCakeMarks {
                    timestamp: now,
                    circuit_hash,
                    cake_marks_down: marks.get_down() as i32,
                    cake_marks_up: marks.get_up() as i32,
                });
            }
        });
        if !cake_drops.is_empty() {
            if lts2_sys::circuit_cake_drops(&cake_drops).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }
        if !cake_marks.is_empty() {
            if lts2_sys::circuit_cake_marks(&cake_marks).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }

        // Network tree stats
        let tree = {
            let reader = NETWORK_JSON.read().unwrap();
            reader.get_nodes_when_ready().clone()
        };
        let mut site_throughput: Vec<lts2_sys::shared_types::SiteThroughput> = Vec::new();
        let mut site_retransmits: Vec<lts2_sys::shared_types::SiteRetransmits> = Vec::new();
        let mut site_rtt: Vec<lts2_sys::shared_types::SiteRtt> = Vec::new();
        let mut site_cake_drops: Vec<lts2_sys::shared_types::SiteCakeDrops> = Vec::new();
        let mut site_cake_marks: Vec<lts2_sys::shared_types::SiteCakeMarks> = Vec::new();
        tree.iter().for_each(|node| {
            let site_hash = hash_to_i64(&node.name);
            if node.current_throughput.not_zero() {
                site_throughput.push(lts2_sys::shared_types::SiteThroughput {
                    timestamp: now,
                    site_hash,
                    download_bytes: scale_u64_by_f64(node.current_throughput.down, scale),
                    upload_bytes: scale_u64_by_f64(node.current_throughput.up, scale),
                    packets_down: scale_u64_by_f64(node.current_packets.down, scale),
                    packets_up: scale_u64_by_f64(node.current_packets.up, scale),
                    packets_tcp_down: scale_u64_by_f64(node.current_tcp_packets.down, scale),
                    packets_tcp_up: scale_u64_by_f64(node.current_tcp_packets.up, scale),
                    packets_udp_down: scale_u64_by_f64(node.current_udp_packets.down, scale),
                    packets_udp_up: scale_u64_by_f64(node.current_udp_packets.up, scale),
                    packets_icmp_down: scale_u64_by_f64(node.current_icmp_packets.down, scale),
                    packets_icmp_up: scale_u64_by_f64(node.current_icmp_packets.up, scale),
                });
            }
            if node.current_tcp_retransmits.not_zero() {
                site_retransmits.push(lts2_sys::shared_types::SiteRetransmits {
                    timestamp: now,
                    site_hash,
                    tcp_retransmits_down: node.current_tcp_retransmits.down as i32,
                    tcp_retransmits_up: node.current_tcp_retransmits.up as i32,
                });
            }
            if node.current_drops.not_zero() {
                site_cake_drops.push(lts2_sys::shared_types::SiteCakeDrops {
                    timestamp: now,
                    site_hash,
                    cake_drops_down: node.current_drops.get_down() as i32,
                    cake_drops_up: node.current_drops.get_up() as i32,
                });
            }
            if node.current_marks.not_zero() {
                site_cake_marks.push(lts2_sys::shared_types::SiteCakeMarks {
                    timestamp: now,
                    site_hash,
                    cake_marks_down: node.current_marks.get_down() as i32,
                    cake_marks_up: node.current_marks.get_up() as i32,
                });
            }
            if !node.rtts.is_empty() {
                let mut rtts: Vec<u16> = node.rtts.iter().map(|n| *n).collect();
                rtts.sort();
                let median = rtts[rtts.len() / 2];

                site_rtt.push(lts2_sys::shared_types::SiteRtt {
                    timestamp: now,
                    site_hash,
                    median_rtt: median as f32 / 10.0,
                });
            }
        });
        if !site_throughput.is_empty() {
            if lts2_sys::site_throughput(&site_throughput).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }
        if !site_retransmits.is_empty() {
            if lts2_sys::site_retransmits(&site_retransmits).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }
        if !site_rtt.is_empty() {
            if lts2_sys::site_rtt(&site_rtt).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }
        if !site_cake_drops.is_empty() {
            if lts2_sys::site_cake_drops(&site_cake_drops).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }
        if !site_cake_marks.is_empty() {
            if lts2_sys::site_cake_marks(&site_cake_marks).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }

        // Shaper utilization
        if counter % 60 == 0 {
            let (tx, rx) = tokio::sync::oneshot::channel();
            if system_usage_actor.send(tx).is_ok() {
                if let Ok(reply) = rx.blocking_recv() {
                    let avg_cpu = reply.cpu_usage.iter().sum::<u32>() as f32 / reply.cpu_usage.len() as f32;
                    let peak_cpu: u32 = reply.cpu_usage.iter().copied().sum();
                    let memory = reply.ram_used as f32 / reply.total_ram as f32;

                    if lts2_sys::shaper_utilization(now, avg_cpu, peak_cpu as f32, memory).is_err() {
                        warn!("Error sending message to LTS2.");
                    }
                }
            }

        }

        // Notify of completion, which triggers processing
        lts2_sys::ingest_batch_complete();
    }
}

fn ip4_to_bytes(ip: (Ipv4Addr, u32)) -> ([u8; 4], u8) {
    let bytes = ip.0.octets();
    (bytes, ip.1 as u8)
}

fn ip6_to_bytes(ip: (Ipv6Addr, u32)) -> ([u8; 16], u8) {
    let bytes = ip.0.octets();
    (bytes, ip.1 as u8)
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
    THROUGHPUT_TRACKER.raw_data.lock().unwrap().iter().for_each(|(k,v)| {
        let ip = k.as_ip();
        result.push((ip, v.bytes_per_second));
    });
    BusResponse::HostCounters(result)
}

#[inline(always)]
fn retire_check(cycle: u64, recent_cycle: u64) -> bool {
    cycle < recent_cycle + RETIRE_AFTER_SECONDS
}

type TopList = (XdpIpAddress, DownUpOrder<u64>,DownUpOrder<u64>, f32, TcHandle, String, DownUpOrder<u64>);

pub fn top_n(start: u32, end: u32) -> BusResponse {
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .unwrap()
            .iter()
            .filter(|(k,v)| !k.as_ip().is_loopback())
            .filter(|(k,d)| retire_check(tp_cycle, d.most_recent_cycle))
            .map(|(k,te)| {
                (
                    *k,
                    te.bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
                    te.tcp_retransmits,
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
            |(
                ip,
                bytes,
                packets,
                median_rtt,
                tc_handle,
                circuit_id,
                tcp_retransmits,      
            )| IpStats {
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

pub fn worst_n(start: u32, end: u32) -> BusResponse {
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .unwrap()
            .iter()
            .filter(|(k,v)| !k.as_ip().is_loopback())
            .filter(|(k,d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(k,te)| te.median_latency().is_some())
            .map(|(k,te)| {
                (
                    *k,
                    te.bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
                    te.tcp_retransmits,
                )
            })
            .collect()
    };
    full_list.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(
                ip,
                bytes,
                packets,
                median_rtt,
                tc_handle,
                circuit_id,
                tcp_retransmits,
            )| IpStats {
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
            .unwrap()
            .iter()
            .filter(|(k,v)| !k.as_ip().is_loopback())
            .filter(|(k,d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(k,te)| te.median_latency().is_some())
            .map(|(k,te)| {
                (
                    *k,
                    te.bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
                    te.tcp_retransmits,
                )
            })
            .collect()
    };
    full_list.sort_by(|a, b| {
        let total_a = a.6.sum();
        let total_b = b.6.sum();
        total_b.cmp(&total_a)
    });
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(
                ip,
                bytes,
                packets,
                median_rtt,
                tc_handle,
                circuit_id,
                tcp_retransmits,
            )| IpStats {
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
            .unwrap()
            .iter()
            .filter(|(k,v)| !k.as_ip().is_loopback())
            .filter(|(k,d)| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|(k,te)| te.median_latency().is_some())
            .map(|(k, te)| {
                (
                    *k,
                    te.bytes_per_second,
                    te.packets_per_second,
                    te.median_latency().unwrap_or(0.0),
                    te.tc_handle,
                    te.circuit_id.as_ref().unwrap_or(&String::new()).clone(),
                    te.tcp_retransmits,
                )
            })
            .collect()
    };
    full_list.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
    full_list.reverse();
    let result = full_list
        .iter()
        //.skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(
                ip,
                bytes,
                packets,
                median_rtt,
                tc_handle,
                circuit_id,
                tcp_retransmits,
            )| IpStats {
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
        .unwrap()
        .iter()
        .filter(|(k,d)| retire_check(raw_cycle, d.most_recent_cycle))
        .filter_map(|(k,data)| {
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
                    let max = *(valid_samples.iter().max().unwrap()) as f32 / 100.0;
                    let min = *(valid_samples.iter().min().unwrap()) as f32 / 100.0;
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
        .unwrap()
        .iter()
        .filter(|(k,d)| retire_check(reader_cycle, d.most_recent_cycle))
        .for_each(|(k,d)| {
            samples.extend(
                d.recent_rtt_data
                    .iter()
                    .filter(|d| d.as_millis() > 0.0)
                    .map(|d| d.as_millis() as f32)
                    .collect::<Vec<f32>>()
            );
        });

    if samples.is_empty() {
        return None;
    }

    // Sort the buffer
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let result = MinMaxMedianRtt {
        min: samples[0] as f32,
        max: samples[samples.len() - 1] as f32,
        median: samples[samples.len() / 2] as f32,
    };

    Some(result)
}

#[derive(Serialize)]
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
    let mut total = TcpRetransmitTotal { up: 0, down: 0, tcp_down: total_tcp.down, tcp_up: total_tcp.up };

    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .unwrap()
        .iter()
        .filter(|(k,d)| retire_check(reader_cycle, d.most_recent_cycle))
        .for_each(|(k,d)| {
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
    for (k,data) in THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .unwrap()
        .iter()
        .filter(|(k,d)| retire_check(reader_cycle, d.most_recent_cycle))
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
            result[usize::min(column, N-1)] += 1;
        }
    }

    BusResponse::RttHistogram(result)
}

pub fn host_counts() -> BusResponse {
    let mut total = 0;
    let mut shaped = 0;
    let tp_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);
    THROUGHPUT_TRACKER
        .raw_data
        .lock()
        .unwrap()
        .iter()
        .filter(|(k,d)| retire_check(tp_cycle, d.most_recent_cycle))
        .for_each(|(k,d)| {
            total += 1;
            if d.tc_handle.as_u32() != 0 {
                shaped += 1;
            }
        });
    BusResponse::HostCounts((total, shaped))
}

type FullList = (XdpIpAddress, DownUpOrder<u64>, DownUpOrder<u64>, f32, TcHandle, u64);

pub fn all_unknown_ips() -> BusResponse {
    let boot_time = time_since_boot();
    if boot_time.is_err() {
        warn!("The Linux system clock isn't available to provide time since boot, yet.");
        warn!("This only happens immediately after a reboot.");
        return BusResponse::NotReadyYet;
    }
    let boot_time = boot_time.unwrap();
    let time_since_boot = Duration::from(boot_time);
    let five_minutes_ago = time_since_boot.saturating_sub(Duration::from_secs(300));
    let five_minutes_ago_nanoseconds = five_minutes_ago.as_nanos();

    let mut full_list: Vec<FullList> = {
        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .unwrap()
            .iter()
            .filter(|(k,v)| !k.as_ip().is_loopback())
            .filter(|(k,d)| d.tc_handle.as_u32() == 0)
            .filter(|(k,d)| d.last_seen as u128 > five_minutes_ago_nanoseconds)
            .map(|(k,te)| {
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
    full_list.sort_by(|a, b| b.5.partial_cmp(&a.5).unwrap());
    let result = full_list
        .iter()
        .map(
            |(
                ip,
                bytes,
                packets,
                median_rtt,
                tc_handle,
                _last_seen,
            )| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: String::new(),
                bits_per_second: bytes.to_bits_from_bytes(),
                packets_per_second: *packets,
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmits: DownUpOrder::zeroed(),
            },
        )
        .collect();
    BusResponse::AllUnknownIps(result)
}

/// For debugging: dump all active flows!
pub fn dump_active_flows() -> BusResponse {
    let lock = ALL_FLOWS.lock().unwrap();
    let result: Vec<lqos_bus::FlowbeeSummaryData> = lock.flow_data
        .iter()
        .map(|(key, row)| {
            let geo =
                get_asn_name_and_country(key.remote_ip.as_ip());

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
                flags: row.0.flags,
                remote_asn: row.1.asn_id.0,
                remote_asn_name: geo.name,
                remote_asn_country: geo.country,
                analysis: row.1.protocol_analysis.to_string(),
                last_seen: row.0.last_seen,
                start_time: row.0.start_time,
                rtt_nanos: DownUpOrder::new(row.0.rtt[0].as_nanos(), row.0.rtt[1].as_nanos()),
                circuit_id,
                circuit_name,
            }
        })
        .collect();

    BusResponse::AllActiveFlows(result)
}

/// Count active flows
pub fn count_active_flows() -> BusResponse {
    let lock = ALL_FLOWS.lock().unwrap();
    BusResponse::CountActiveFlows(lock.flow_data.len() as u64)
}

/// Top Flows Report
pub fn top_flows(n: u32, flow_type: TopFlowType) -> BusResponse {
    let lock = ALL_FLOWS.lock().unwrap();
    let mut table: Vec<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))> = lock.flow_data
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    std::mem::drop(lock); // Early lock release

    match flow_type {
        TopFlowType::RateEstimate => {
            table.sort_by(|a, b| {
                let a_total = a.1 .0.rate_estimate_bps.sum();
                let b_total = b.1 .0.rate_estimate_bps.sum();
                b_total.cmp(&a_total)
            });
        }
        TopFlowType::Bytes => {
            table.sort_by(|a, b| {
                let a_total = a.1 .0.bytes_sent.sum();
                let b_total = b.1 .0.bytes_sent.sum();
                b_total.cmp(&a_total)
            });
        }
        TopFlowType::Packets => {
            table.sort_by(|a, b| {
                let a_total = a.1 .0.packets_sent.sum();
                let b_total = b.1 .0.packets_sent.sum();
                b_total.cmp(&a_total)
            });
        }
        TopFlowType::Drops => {
            table.sort_by(|a, b| {
                let a_total = a.1 .0.tcp_retransmits.sum();
                let b_total = b.1 .0.tcp_retransmits.sum();
                b_total.cmp(&a_total)
            });
        }
        TopFlowType::RoundTripTime => {
            table.sort_by(|a, b| {
                let a_total = a.1 .0.rtt;
                let b_total = b.1 .0.rtt;
                a_total.cmp(&b_total)
            });
        }
    }

    let sd = SHAPED_DEVICES.load();

    let result = table
        .iter()
        .take(n as usize)
        .map(|(ip, flow)| {
            let geo =
                get_asn_name_and_country(ip.remote_ip.as_ip());

            let (circuit_id, circuit_name) = sd.get_circuit_id_and_name_from_ip(&ip.local_ip).unwrap_or((String::new(), String::new()));

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
                flags: flow.0.flags,
                remote_asn: flow.1.asn_id.0,
                remote_asn_name: geo.name,
                remote_asn_country: geo.country,
                analysis: flow.1.protocol_analysis.to_string(),
                last_seen: flow.0.last_seen,
                start_time: flow.0.start_time,
                rtt_nanos: DownUpOrder::new(flow.0.rtt[0].as_nanos(), flow.0.rtt[1].as_nanos()),
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
        let lock = ALL_FLOWS.lock().unwrap();
        let sd = SHAPED_DEVICES.load();
        let matching_flows: Vec<_> = lock.flow_data
            .iter()
            .filter(|(key, _)| key.local_ip == ip)
            .map(|(key, row)| {
                let geo =
                    get_asn_name_and_country(key.remote_ip.as_ip());

                let (circuit_id, circuit_name) = sd.get_circuit_id_and_name_from_ip(&key.local_ip).unwrap_or((String::new(), String::new()));

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
                    flags: row.0.flags,
                    remote_asn: row.1.asn_id.0,
                    remote_asn_name: geo.name,
                    remote_asn_country: geo.country,
                    analysis: row.1.protocol_analysis.to_string(),
                    last_seen: row.0.last_seen,
                    start_time: row.0.start_time,
                    rtt_nanos: DownUpOrder::new(row.0.rtt[0].as_nanos(), row.0.rtt[1].as_nanos()),
                    circuit_id,
                    circuit_name,
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
    BusResponse::IpProtocols(
        flow_data::RECENT_FLOWS.ip_protocol_summary()
    )
}

/// Flow duration summary
pub fn flow_duration() -> BusResponse {
    BusResponse::FlowDuration(
        flow_data::RECENT_FLOWS.flow_duration_summary()
            .into_iter()
            .map(|v| (v.count, v.duration))
            .collect()
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
    children: Vec<Lts2NetJs>
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