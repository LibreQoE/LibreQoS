pub mod flow_data;
mod throughput_entry;
mod tracking_data;
use std::net::IpAddr;

use self::flow_data::{get_asn_name_and_country, FlowAnalysis, FlowbeeLocalData, ALL_FLOWS};
use crate::{
    long_term_stats::get_network_tree,
    shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES, STATS_NEEDS_NEW_SHAPED_DEVICES},
    stats::TIME_TO_POLL_HOSTS,
    throughput_tracker::tracking_data::ThroughputTracker,
};
use log::{info, warn};
use lqos_bus::{BusResponse, FlowbeeProtocol, IpStats, TcHandle, TopFlowType, XdpPpingResult};
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::{unix_time::time_since_boot, XdpIpAddress};
use lts_client::collector::{HostSummary, StatsUpdateMessage, ThroughputSummary};
use once_cell::sync::Lazy;
use tokio::{
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

const RETIRE_AFTER_SECONDS: u64 = 30;

pub static THROUGHPUT_TRACKER: Lazy<ThroughputTracker> = Lazy::new(ThroughputTracker::new);

/// Create the throughput monitor thread, and begin polling for
/// throughput data every second.
///
/// ## Arguments
///
/// * `long_term_stats_tx` - an optional MPSC sender to notify the
///   collection thread that there is fresh data.
pub async fn spawn_throughput_monitor(
    long_term_stats_tx: Sender<StatsUpdateMessage>,
    netflow_sender: std::sync::mpsc::Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
) {
    info!("Starting the bandwidth monitor thread.");
    let interval_ms = 1000; // 1 second
    info!("Bandwidth check period set to {interval_ms} ms.");
    tokio::spawn(throughput_task(
        interval_ms,
        long_term_stats_tx,
        netflow_sender,
    ));
}

async fn throughput_task(
    interval_ms: u64,
    long_term_stats_tx: Sender<StatsUpdateMessage>,
    netflow_sender: std::sync::mpsc::Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>,
) {
    // Obtain the flow timeout from the config, default to 30 seconds
    let timeout_seconds = if let Ok(config) = lqos_config::load_config() {
        if let Some(flow_config) = config.flows {
            flow_config.flow_timeout_seconds
        } else {
            30
        }
    } else {
        30
    };

    // Obtain the netflow_enabled from the config, default to false
    let netflow_enabled = if let Ok(config) = lqos_config::load_config() {
        if let Some(flow_config) = config.flows {
            flow_config.netflow_enabled
        } else {
            false
        }
    } else {
        false
    };

    loop {
        let start = Instant::now();

        // Perform the stats collection in a blocking thread, ensuring that
        // the tokio runtime is not blocked.
        let my_netflow_sender = netflow_sender.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || {
            {
                let net_json = NETWORK_JSON.read().unwrap();
                net_json.zero_throughput_and_rtt();
            } // Scope to end the lock
            THROUGHPUT_TRACKER.copy_previous_and_reset_rtt();
            THROUGHPUT_TRACKER.apply_new_throughput_counters();
            THROUGHPUT_TRACKER.apply_flow_data(
                timeout_seconds,
                netflow_enabled,
                my_netflow_sender.clone(),
            );
            THROUGHPUT_TRACKER.update_totals();
            THROUGHPUT_TRACKER.next_cycle();
            let duration_ms = start.elapsed().as_micros();
            TIME_TO_POLL_HOSTS.store(duration_ms as u64, std::sync::atomic::Ordering::Relaxed);
        })
        .await
        {
            log::error!("Error polling network. {e:?}");
        }
        tokio::spawn(submit_throughput_stats(long_term_stats_tx.clone()));

        let elapsed = start.elapsed();
        if elapsed.as_secs_f32() < 1.0 {
            let sleep_duration = Duration::from_millis(interval_ms) - start.elapsed();
            tokio::time::sleep(sleep_duration).await;
        } else {
            log::error!("Throughput monitor thread is running behind. It took {elapsed} to poll the network.", elapsed=elapsed.as_secs_f32());
        }
    }
}

async fn submit_throughput_stats(long_term_stats_tx: Sender<StatsUpdateMessage>) {
    // If ShapedDevices has changed, notify the stats thread
    if let Ok(changed) = STATS_NEEDS_NEW_SHAPED_DEVICES.compare_exchange(
        true,
        false,
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
    ) {
        if changed {
            let shaped_devices = SHAPED_DEVICES.read().unwrap().devices.clone();
            let _ = long_term_stats_tx
                .send(StatsUpdateMessage::ShapedDevicesChanged(shaped_devices))
                .await;
        }
    }

    // Gather Global Stats
    let packets_per_second = (
        THROUGHPUT_TRACKER
            .packets_per_second
            .0
            .load(std::sync::atomic::Ordering::Relaxed),
        THROUGHPUT_TRACKER
            .packets_per_second
            .1
            .load(std::sync::atomic::Ordering::Relaxed),
    );
    let bits_per_second = THROUGHPUT_TRACKER.bits_per_second();
    let shaped_bits_per_second = THROUGHPUT_TRACKER.shaped_bits_per_second();
    let hosts = THROUGHPUT_TRACKER
        .raw_data
        .iter()
        .filter(|host| host.median_latency().is_some())
        .map(|host| HostSummary {
            ip: host.key().as_ip(),
            circuit_id: host.circuit_id.clone(),
            bits_per_second: (host.bytes_per_second.0 * 8, host.bytes_per_second.1 * 8),
            median_rtt: host.median_latency().unwrap_or(0.0),
        })
        .collect();

    let summary = Box::new((
        ThroughputSummary {
            bits_per_second,
            shaped_bits_per_second,
            packets_per_second,
            hosts,
        },
        get_network_tree(),
    ));

    // Send the stats
    let result = long_term_stats_tx
        .send(StatsUpdateMessage::ThroughputReady(summary))
        .await;
    if let Err(e) = result {
        warn!("Error sending message to stats collection system. {e:?}");
    }
}

pub fn current_throughput() -> BusResponse {
    let (bits_per_second, packets_per_second, shaped_bits_per_second) = {
        (
            THROUGHPUT_TRACKER.bits_per_second(),
            THROUGHPUT_TRACKER.packets_per_second(),
            THROUGHPUT_TRACKER.shaped_bits_per_second(),
        )
    };
    BusResponse::CurrentThroughput {
        bits_per_second,
        packets_per_second,
        shaped_bits_per_second,
    }
}

pub fn host_counters() -> BusResponse {
    let mut result = Vec::new();
    THROUGHPUT_TRACKER.raw_data.iter().for_each(|v| {
        let ip = v.key().as_ip();
        let (down, up) = v.bytes_per_second;
        result.push((ip, down, up));
    });
    BusResponse::HostCounters(result)
}

#[inline(always)]
fn retire_check(cycle: u64, recent_cycle: u64) -> bool {
    cycle < recent_cycle + RETIRE_AFTER_SECONDS
}

type TopList = (XdpIpAddress, (u64, u64), (u64, u64), f32, TcHandle, String, (u64, u64));

pub fn top_n(start: u32, end: u32) -> BusResponse {
    let mut full_list: Vec<TopList> = {
        let tp_cycle = THROUGHPUT_TRACKER
            .cycle
            .load(std::sync::atomic::Ordering::Relaxed);
        THROUGHPUT_TRACKER
            .raw_data
            .iter()
            .filter(|v| !v.key().as_ip().is_loopback())
            .filter(|d| retire_check(tp_cycle, d.most_recent_cycle))
            .map(|te| {
                (
                    *te.key(),
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
    full_list.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    let result = full_list
        .iter()
        .skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(
                ip,
                (bytes_dn, bytes_up),
                (packets_dn, packets_up),
                median_rtt,
                tc_handle,
                circuit_id,
                tcp_retransmits,      
            )| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: (bytes_dn * 8, bytes_up * 8),
                packets_per_second: (*packets_dn, *packets_up),
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
            .iter()
            .filter(|v| !v.key().as_ip().is_loopback())
            .filter(|d| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|te| te.median_latency().is_some())
            .map(|te| {
                (
                    *te.key(),
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
        .skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(
                ip,
                (bytes_dn, bytes_up),
                (packets_dn, packets_up),
                median_rtt,
                tc_handle,
                circuit_id,
                tcp_retransmits,
            )| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: (bytes_dn * 8, bytes_up * 8),
                packets_per_second: (*packets_dn, *packets_up),
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
            .iter()
            .filter(|v| !v.key().as_ip().is_loopback())
            .filter(|d| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|te| te.median_latency().is_some())
            .map(|te| {
                (
                    *te.key(),
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
        let total_a = a.6 .0 + a.6 .1;
        let total_b = b.6 .0 + b.6 .1;
        total_b.cmp(&total_a)
    });
    let result = full_list
        .iter()
        .skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(
                ip,
                (bytes_dn, bytes_up),
                (packets_dn, packets_up),
                median_rtt,
                tc_handle,
                circuit_id,
                tcp_retransmits,
            )| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: (bytes_dn * 8, bytes_up * 8),
                packets_per_second: (*packets_dn, *packets_up),
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
            .iter()
            .filter(|v| !v.key().as_ip().is_loopback())
            .filter(|d| retire_check(tp_cycle, d.most_recent_cycle))
            .filter(|te| te.median_latency().is_some())
            .map(|te| {
                (
                    *te.key(),
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
        .skip(start as usize)
        .take((end as usize) - (start as usize))
        .map(
            |(
                ip,
                (bytes_dn, bytes_up),
                (packets_dn, packets_up),
                median_rtt,
                tc_handle,
                circuit_id,
                tcp_retransmits,
            )| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: circuit_id.clone(),
                bits_per_second: (bytes_dn * 8, bytes_up * 8),
                packets_per_second: (*packets_dn, *packets_up),
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
        .iter()
        .filter(|d| retire_check(raw_cycle, d.most_recent_cycle))
        .filter_map(|data| {
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

pub fn rtt_histogram() -> BusResponse {
    let mut result = vec![0; 20];
    let reader_cycle = THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);
    for data in THROUGHPUT_TRACKER
        .raw_data
        .iter()
        .filter(|d| retire_check(reader_cycle, d.most_recent_cycle))
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
            let median = f32::min(200.0, median);
            let column = median as usize;
            result[usize::min(column, 19)] += 1;
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
        .iter()
        .filter(|d| retire_check(tp_cycle, d.most_recent_cycle))
        .for_each(|d| {
            total += 1;
            if d.tc_handle.as_u32() != 0 {
                shaped += 1;
            }
        });
    BusResponse::HostCounts((total, shaped))
}

type FullList = (XdpIpAddress, (u64, u64), (u64, u64), f32, TcHandle, u64);

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
            .iter()
            .filter(|v| !v.key().as_ip().is_loopback())
            .filter(|d| d.tc_handle.as_u32() == 0)
            .filter(|d| d.last_seen as u128 > five_minutes_ago_nanoseconds)
            .map(|te| {
                (
                    *te.key(),
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
                (bytes_dn, bytes_up),
                (packets_dn, packets_up),
                median_rtt,
                tc_handle,
                _last_seen,
            )| IpStats {
                ip_address: ip.as_ip().to_string(),
                circuit_id: String::new(),
                bits_per_second: (bytes_dn * 8, bytes_up * 8),
                packets_per_second: (*packets_dn, *packets_up),
                median_tcp_rtt: *median_rtt,
                tc_handle: *tc_handle,
                tcp_retransmits: (0, 0),
            },
        )
        .collect();
    BusResponse::AllUnknownIps(result)
}

/// For debugging: dump all active flows!
pub fn dump_active_flows() -> BusResponse {
    let lock = ALL_FLOWS.lock().unwrap();
    let result: Vec<lqos_bus::FlowbeeSummaryData> = lock
        .iter()
        .map(|(key, row)| {
            let (remote_asn_name, remote_asn_country) =
                get_asn_name_and_country(key.remote_ip.as_ip());

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
                remote_asn_name,
                remote_asn_country,
                analysis: row.1.protocol_analysis.to_string(),
                last_seen: row.0.last_seen,
                start_time: row.0.start_time,
                rtt_nanos: [row.0.rtt[0].as_nanos(), row.0.rtt[1].as_nanos()],
            }
        })
        .collect();

    BusResponse::AllActiveFlows(result)
}

/// Count active flows
pub fn count_active_flows() -> BusResponse {
    let lock = ALL_FLOWS.lock().unwrap();
    BusResponse::CountActiveFlows(lock.len() as u64)
}

/// Top Flows Report
pub fn top_flows(n: u32, flow_type: TopFlowType) -> BusResponse {
    let lock = ALL_FLOWS.lock().unwrap();
    let mut table: Vec<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))> = lock
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    std::mem::drop(lock); // Early lock release

    match flow_type {
        TopFlowType::RateEstimate => {
            table.sort_by(|a, b| {
                let a_total = a.1 .0.rate_estimate_bps[0] + a.1 .0.rate_estimate_bps[1];
                let b_total = b.1 .0.rate_estimate_bps[0] + b.1 .0.rate_estimate_bps[1];
                b_total.cmp(&a_total)
            });
        }
        TopFlowType::Bytes => {
            table.sort_by(|a, b| {
                let a_total = a.1 .0.bytes_sent[0] + a.1 .0.bytes_sent[1];
                let b_total = b.1 .0.bytes_sent[0] + b.1 .0.bytes_sent[1];
                b_total.cmp(&a_total)
            });
        }
        TopFlowType::Packets => {
            table.sort_by(|a, b| {
                let a_total = a.1 .0.packets_sent[0] + a.1 .0.packets_sent[1];
                let b_total = b.1 .0.packets_sent[0] + b.1 .0.packets_sent[1];
                b_total.cmp(&a_total)
            });
        }
        TopFlowType::Drops => {
            table.sort_by(|a, b| {
                let a_total = a.1 .0.tcp_retransmits[0] + a.1 .0.tcp_retransmits[1];
                let b_total = b.1 .0.tcp_retransmits[0] + b.1 .0.tcp_retransmits[1];
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

    let result = table
        .iter()
        .take(n as usize)
        .map(|(ip, flow)| {
            let (remote_asn_name, remote_asn_country) =
                get_asn_name_and_country(ip.remote_ip.as_ip());
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
                remote_asn_name,
                remote_asn_country,
                analysis: flow.1.protocol_analysis.to_string(),
                last_seen: flow.0.last_seen,
                start_time: flow.0.start_time,
                rtt_nanos: [flow.0.rtt[0].as_nanos(), flow.0.rtt[1].as_nanos()],
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
        let matching_flows: Vec<_> = lock
            .iter()
            .filter(|(key, _)| key.local_ip == ip)
            .map(|(key, row)| {
                let (remote_asn_name, remote_asn_country) =
                    get_asn_name_and_country(key.remote_ip.as_ip());

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
                    remote_asn_name,
                    remote_asn_country,
                    analysis: row.1.protocol_analysis.to_string(),
                    last_seen: row.0.last_seen,
                    start_time: row.0.start_time,
                    rtt_nanos: [row.0.rtt[0].as_nanos(), row.0.rtt[1].as_nanos()],
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
