use std::fs::read_to_string;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use fxhash::FxHashMap;
use tokio::sync::mpsc::Sender;
use tokio::time::Instant;
use tracing::debug;
use tracing::log::warn;
use lqos_config::load_config;
use lqos_queue_tracker::{ALL_QUEUE_SUMMARY, TOTAL_QUEUE_STATS};
use lqos_utils::hash_to_i64;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::unix_now;
use lts2_sys::shared_types::{CircuitCakeDrops, CircuitCakeMarks};
use lts_client::collector::{HostSummary, ThroughputSummary};
use lts_client::collector::stats_availability::StatsUpdateMessage;
use crate::long_term_stats::get_network_tree;
use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES, STATS_NEEDS_NEW_SHAPED_DEVICES};
use crate::system_stats::SystemStats;
use crate::throughput_tracker::{min_max_median_rtt, min_max_median_tcp_retransmits, Lts2Circuit, Lts2Device, RawNetJs, THROUGHPUT_TRACKER};
use crate::throughput_tracker::flow_data::ALL_FLOWS;

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

pub(crate) fn submit_throughput_stats(
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

    // Check if we should be submitting to Insight
    let Ok(config) = load_config() else { return; };
    if config.long_term_stats.use_insight.unwrap_or(false) == false {
        return;
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
                                        warn!("Error sending message to Insight. {e:?}");
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

            // TODO: Send permitted IP ranges at the same time
            if let Ok(config) = lqos_config::load_config() {
                lts2_sys::ip_policies(
                    &config.ip_ranges.allow_subnets,
                    &config.ip_ranges.ignore_subnets
                );
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
        lts2_sys::flow_count(now, ALL_FLOWS.lock().unwrap().flow_data.len() as u64);

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
            .filter(|(_k,h)| h.circuit_id.is_some() && h.bytes_per_second.not_zero())
            .for_each(|(_k,h)| {
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
            .filter(|(_k,h)| h.circuit_id.is_some() && h.tcp_retransmits.not_zero())
            .for_each(|(_k,h)| {
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
            .filter(|(_k,h)| h.circuit_id.is_some() && h.median_latency().is_some())
            .for_each(|(_k,h)| {
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

                    if let Err(e) = lts2_sys::shaper_utilization(now, avg_cpu, peak_cpu as f32, memory) {
                        warn!("Error sending message to LTS2: {e:?}");
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
