use crate::lts2_sys::get_lts_license_status;
use crate::lts2_sys::shared_types::{CircuitCakeDrops, CircuitCakeMarks, LtsStatus};
use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES};
use crate::system_stats::SystemStats;
use crate::throughput_tracker::flow_data::ALL_FLOWS;
use crate::throughput_tracker::flow_data::FlowbeeEffectiveDirection;
use crate::throughput_tracker::{
    CIRCUIT_RTT_BUFFERS, Lts2Circuit, Lts2Device, RawNetJs, THROUGHPUT_TRACKER, min_max_median_rtt,
    min_max_median_tcp_retransmits,
};
use csv::ReaderBuilder;
use fxhash::{FxHashMap, FxHashSet};
use lqos_config::{ShapedDevice, load_config};
use lqos_queue_tracker::{ALL_QUEUE_SUMMARY, TOTAL_QUEUE_STATS};
use lqos_utils::hash_to_i64;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::unix_now;
use std::fs::read_to_string;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::atomic::AtomicI64;
use tracing::debug;
use tracing::log::warn;
use uuid::Uuid;

fn scale_u64_by_f64(value: u64, scale: f64) -> u64 {
    (value as f64 * scale) as u64
}

/// Temporary conversion function for LTS/Insight compatibility
/// TODO: Remove when LTS/Insight support fractional rates
fn rate_for_submission(rate_mbps: f32) -> u32 {
    if rate_mbps < 1.0 {
        1 // Round up small fractional rates to 1 Mbps for now
    } else {
        rate_mbps.round() as u32 // Round to nearest integer
    }
}

pub(crate) fn submit_throughput_stats(
    scale: f64,
    counter: u8,
    system_usage_actor: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) {
    // Load the config
    let Ok(config) = load_config() else {
        return;
    };

    // Bail out if we don't have gather stats or a license key
    if config.long_term_stats.gather_stats == false {
        return;
    }
    if let Some(license_key) = &config.long_term_stats.license_key {
        if license_key.trim().is_empty() {
            // There's a license key but it's empty
            return;
        }
        if license_key.trim().replace("-", "").parse::<Uuid>().is_err() {
            return; // Invalid license key format
        }
    } else {
        return;
    }

    // Bail out if the license doesn't indicate that we're allowed to submit stats
    let (license_status, _days_remaining) = get_lts_license_status();
    let can_submit = match license_status {
        LtsStatus::NotChecked | LtsStatus::ApiOnly | LtsStatus::Invalid => false,
        _ => true,
    };
    if !can_submit {
        return;
    }

    // Gather Global Stats
    let packets_per_second = (
        THROUGHPUT_TRACKER.packets_per_second.get_down(),
        THROUGHPUT_TRACKER.packets_per_second.get_up(),
    );
    let tcp_packets_per_second = (
        THROUGHPUT_TRACKER.tcp_packets_per_second.get_down(),
        THROUGHPUT_TRACKER.tcp_packets_per_second.get_up(),
    );
    let udp_packets_per_second = (
        THROUGHPUT_TRACKER.udp_packets_per_second.get_down(),
        THROUGHPUT_TRACKER.udp_packets_per_second.get_up(),
    );
    let icmp_packets_per_second = (
        THROUGHPUT_TRACKER.icmp_packets_per_second.get_down(),
        THROUGHPUT_TRACKER.icmp_packets_per_second.get_up(),
    );
    let bits_per_second = THROUGHPUT_TRACKER.bits_per_second();

    // Check that the stats haven't gone wonky and don't submit obviously bad data.
    if let Ok(config) = load_config() {
        if bits_per_second.down > (config.queues.downlink_bandwidth_mbps * 1_000_000) {
            debug!("Spike detected - not submitting LTS");
            return; // Do not submit these stats
        }
        if bits_per_second.up > (config.queues.uplink_bandwidth_mbps * 1_000_000) {
            debug!("Spike detected - not submitting LTS");
            return; // Do not submit these stats
        }
    }

    /////////////////////////////////////////////////////////////////
    // Insight Block
    if let Ok(now) = unix_now() {
        // LTS2 Shaped Devices
        if lts2_needs_shaped_devices() {
            tracing::info!("Sending topology to Insight");
            // Send the topology tree
            {
                let filename = Path::new(&config.lqos_directory).join("network.json");
                if let Ok(raw_string) = read_to_string(filename) {
                    match serde_json::from_str::<RawNetJs>(&raw_string) {
                        Err(e) => {
                            warn!("Unable to parse network.json. {e:?}");
                        }
                        Ok(json) => {
                            let lts2_format: Vec<_> =
                                json.iter().map(|(k, v)| v.to_lts2(&k)).collect();
                            if let Ok(bytes) = serde_cbor::to_vec(&lts2_format) {
                                if let Err(e) = crate::lts2_sys::network_tree(now, &bytes) {
                                    warn!("Error sending message to Insight. {e:?}");
                                }
                            }
                        }
                    }
                } else {
                    warn!("Unable to read network.json");
                }
            }

            // Send the shaped devices
            let shaped_devices = load_local_shaped_devices();
            tracing::info!("Loaded local shaped devices");
            if let Ok(shaped_devices) = shaped_devices {
                tracing::info!("And they are good");
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
                                download_min_mbps: rate_for_submission(device.download_min_mbps),
                                upload_min_mbps: rate_for_submission(device.upload_min_mbps),
                                download_max_mbps: rate_for_submission(device.download_max_mbps),
                                upload_max_mbps: rate_for_submission(device.upload_max_mbps),
                                parent_node: device.parent_hash,
                                parent_node_name: Some(device.parent_node),
                                devices: vec![Lts2Device {
                                    device_hash: device.device_hash,
                                    device_id: device.device_id,
                                    device_name: device.device_name,
                                    mac: device.mac,
                                    ipv4: device.ipv4.into_iter().map(ip4_to_bytes).collect(),
                                    ipv6: device.ipv6.into_iter().map(ip6_to_bytes).collect(),
                                    comment: device.comment,
                                }],
                            },
                        );
                    }
                }
                let devices_as_vec: Vec<Lts2Circuit> =
                    circuit_map.into_iter().map(|(_, v)| v).collect();
                // Serialize via cbor
                if let Ok(bytes) = serde_cbor::to_vec(&devices_as_vec) {
                    if crate::lts2_sys::shaped_devices(now, &bytes).is_err() {
                        warn!("Error sending message to LTS2.");
                    }
                }
            }

            // Send permitted IP ranges at the same time
            if let Ok(config) = lqos_config::load_config() {
                if let Err(e) = crate::lts2_sys::ip_policies(
                    &config.ip_ranges.allow_subnets,
                    &config.ip_ranges.ignore_subnets,
                ) {
                    debug!("Error sending message to LTS2. {e:?}");
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
        if crate::lts2_sys::total_throughput(
            now,
            scale_u64_by_f64(bytes.down, scale),
            scale_u64_by_f64(bytes.up, scale),
            scale_u64_by_f64(shaped_bytes.down, scale),
            scale_u64_by_f64(shaped_bytes.up, scale),
            scale_u64_by_f64(packets_per_second.0, scale),
            scale_u64_by_f64(packets_per_second.1, scale),
            scale_u64_by_f64(tcp_packets_per_second.0, scale),
            scale_u64_by_f64(tcp_packets_per_second.1, scale),
            scale_u64_by_f64(udp_packets_per_second.0, scale),
            scale_u64_by_f64(udp_packets_per_second.1, scale),
            scale_u64_by_f64(icmp_packets_per_second.0, scale),
            scale_u64_by_f64(icmp_packets_per_second.1, scale),
            min_rtt,
            max_rtt,
            median_rtt,
            tcp_retransmits.down,
            tcp_retransmits.up,
            TOTAL_QUEUE_STATS.marks.get_down() as i32,
            TOTAL_QUEUE_STATS.marks.get_up() as i32,
            TOTAL_QUEUE_STATS.drops.get_down() as i32,
            TOTAL_QUEUE_STATS.drops.get_up() as i32,
        )
        .is_err()
        {
            warn!("Error sending message to LTS2.");
        }
        if let Err(e) = crate::lts2_sys::flow_count(now, ALL_FLOWS.lock().flow_data.len() as u64) {
            debug!("Error sending message to LTS2. {e:?}");
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

        let mut crazy_values: FxHashSet<i64> = FxHashSet::default(); // Circuits to skip because the numbers are too high
        let mut circuit_throughput: FxHashMap<i64, CircuitThroughputTemp> = FxHashMap::default();
        let mut circuit_retransmits: FxHashMap<i64, DownUpOrder<u64>> = FxHashMap::default();

        let shaped_devices = SHAPED_DEVICES.load();
        const CRAZY_LIMIT: u64 = 8; // 8x the max bandwidth
        let plan_lookup: FxHashMap<i64, (u64, u64)> = shaped_devices
            .devices
            .iter()
            // Bandwidth: mbps * 1_000_000 to bytes
            .map(|d| {
                (
                    d.circuit_hash,
                    (
                        d.download_max_mbps.round() as u64 * 1_000_000 * CRAZY_LIMIT,
                        d.upload_max_mbps.round() as u64 * 1_000_000 * CRAZY_LIMIT,
                    ),
                )
            })
            .collect();

        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(_k, h)| h.circuit_id.is_some() && h.bytes_per_second.not_zero())
            .for_each(|(_k, h)| {
                let mut crazy = false;
                if let Some((dl, ul)) = plan_lookup.get(&h.circuit_hash.unwrap_or(0)) {
                    if h.bytes_per_second.down > *dl {
                        crazy_values.insert(h.circuit_hash.unwrap_or(0));
                        crazy = true;
                    } else if h.bytes_per_second.up > *ul {
                        crazy_values.insert(h.circuit_hash.unwrap_or(0));
                        crazy = true;
                    }
                }

                if crazy {
                    return;
                }
                if let Some(c) = circuit_throughput.get_mut(&h.circuit_hash.unwrap_or(0)) {
                    c.bytes += h.bytes_per_second;
                    c.packets += h.packets_per_second;
                    c.tcp_packets += h.tcp_packets;
                    c.udp_packets += h.udp_packets;
                    c.icmp_packets += h.icmp_packets;
                } else {
                    circuit_throughput.insert(
                        h.circuit_hash.unwrap_or(0),
                        CircuitThroughputTemp {
                            bytes: h.bytes_per_second,
                            packets: h.packets_per_second,
                            tcp_packets: h.tcp_packets,
                            udp_packets: h.udp_packets,
                            icmp_packets: h.icmp_packets,
                        },
                    );
                }
            });

        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter(|(_k, h)| {
                h.circuit_id.is_some()
                    && h.tcp_retransmits.not_zero()
                    && !crazy_values.contains(&h.circuit_hash.unwrap_or(0))
            })
            .for_each(|(_k, h)| {
                if let Some(c) = circuit_retransmits.get_mut(&h.circuit_hash.unwrap_or(0)) {
                    *c += h.tcp_retransmits;
                } else {
                    circuit_retransmits.insert(h.circuit_hash.unwrap_or(0), h.tcp_retransmits);
                }
            });

        // And now we send it
        let circuit_throughput_batch = circuit_throughput
            .into_iter()
            .map(|(k, v)| crate::lts2_sys::shared_types::CircuitThroughput {
                timestamp: now,
                circuit_hash: k,
                download_bytes: scale_u64_by_f64(v.bytes.down, scale),
                upload_bytes: scale_u64_by_f64(v.bytes.up, scale),
                packets_down: scale_u64_by_f64(v.packets.down, scale),
                packets_up: scale_u64_by_f64(v.packets.up, scale),
                tcp_packets_down: scale_u64_by_f64(v.tcp_packets.down, scale),
                tcp_packets_up: scale_u64_by_f64(v.tcp_packets.up, scale),
                udp_packets_down: scale_u64_by_f64(v.udp_packets.down, scale),
                udp_packets_up: scale_u64_by_f64(v.udp_packets.up, scale),
                icmp_packets_down: scale_u64_by_f64(v.icmp_packets.down, scale),
                icmp_packets_up: scale_u64_by_f64(v.icmp_packets.up, scale),
            })
            .collect::<Vec<_>>();
        if crate::lts2_sys::circuit_throughput(&circuit_throughput_batch).is_err() {
            warn!("Error sending message to LTS2.");
        }

        let circuit_retransmits_batch = circuit_retransmits
            .into_iter()
            .map(|(k, v)| crate::lts2_sys::shared_types::CircuitRetransmits {
                timestamp: now,
                circuit_hash: k,
                tcp_retransmits_down: v.down as u32,
                tcp_retransmits_up: v.up as u32,
            })
            .collect::<Vec<_>>();
        if crate::lts2_sys::circuit_retransmits(&circuit_retransmits_batch).is_err() {
            warn!("Error sending message to LTS2.");
        }

        let circuit_rtt_snapshot = CIRCUIT_RTT_BUFFERS.load();
        let circuit_rtt_batch = circuit_rtt_snapshot
            .iter()
            .filter(|(circuit_hash, _)| !crazy_values.contains(circuit_hash))
            .filter_map(|(circuit_hash, rtt_buffer)| {
                let download = rtt_buffer
                    .median_new_data(FlowbeeEffectiveDirection::Download)
                    .as_nanos();
                let upload = rtt_buffer
                    .median_new_data(FlowbeeEffectiveDirection::Upload)
                    .as_nanos();

                let median_nanos = match (download, upload) {
                    (0, 0) => return None,
                    (d, 0) => d,
                    (0, u) => u,
                    (d, u) => d.saturating_add(u) / 2,
                };

                Some(crate::lts2_sys::shared_types::CircuitRtt {
                    timestamp: now,
                    circuit_hash: *circuit_hash,
                    median_rtt: (median_nanos as f64 / 1_000_000.0) as f32,
                })
            })
            .collect::<Vec<_>>();
        if crate::lts2_sys::circuit_rtt(&circuit_rtt_batch).is_err() {
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
                    cake_drops_down: drops.get_down() as u32,
                    cake_drops_up: drops.get_up() as u32,
                });
            }
            if marks.not_zero() {
                cake_marks.push(CircuitCakeMarks {
                    timestamp: now,
                    circuit_hash,
                    cake_marks_down: marks.get_down() as u32,
                    cake_marks_up: marks.get_up() as u32,
                });
            }
        });
        if !cake_drops.is_empty() {
            if crate::lts2_sys::circuit_cake_drops(&cake_drops).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }
        if !cake_marks.is_empty() {
            if crate::lts2_sys::circuit_cake_marks(&cake_marks).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }

        // Network tree stats
        let tree = {
            let reader = NETWORK_JSON.read();
            reader.get_nodes_when_ready().clone()
        };
        let mut site_throughput: Vec<crate::lts2_sys::shared_types::SiteThroughput> = Vec::new();
        let mut site_retransmits: Vec<crate::lts2_sys::shared_types::SiteRetransmits> = Vec::new();
        let mut site_rtt: Vec<crate::lts2_sys::shared_types::SiteRtt> = Vec::new();
        let mut site_cake_drops: Vec<crate::lts2_sys::shared_types::SiteCakeDrops> = Vec::new();
        let mut site_cake_marks: Vec<crate::lts2_sys::shared_types::SiteCakeMarks> = Vec::new();
        tree.iter().for_each(|node| {
            let site_hash = hash_to_i64(&node.name);
            if node.current_throughput.not_zero() {
                site_throughput.push(crate::lts2_sys::shared_types::SiteThroughput {
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
                site_retransmits.push(crate::lts2_sys::shared_types::SiteRetransmits {
                    timestamp: now,
                    site_hash,
                    tcp_retransmits_down: node.current_tcp_retransmits.down as u32,
                    tcp_retransmits_up: node.current_tcp_retransmits.up as u32,
                });
            }
            if node.current_drops.not_zero() {
                site_cake_drops.push(crate::lts2_sys::shared_types::SiteCakeDrops {
                    timestamp: now,
                    site_hash,
                    cake_drops_down: node.current_drops.get_down() as u32,
                    cake_drops_up: node.current_drops.get_up() as u32,
                });
            }
            if node.current_marks.not_zero() {
                site_cake_marks.push(crate::lts2_sys::shared_types::SiteCakeMarks {
                    timestamp: now,
                    site_hash,
                    cake_marks_down: node.current_marks.get_down() as u32,
                    cake_marks_up: node.current_marks.get_up() as u32,
                });
            }
            let download = node
                .rtt_buffer
                .median_new_data(FlowbeeEffectiveDirection::Download)
                .as_nanos();
            let upload = node
                .rtt_buffer
                .median_new_data(FlowbeeEffectiveDirection::Upload)
                .as_nanos();
            let median_nanos = match (download, upload) {
                (0, 0) => None,
                (d, 0) => Some(d),
                (0, u) => Some(u),
                (d, u) => Some(d.saturating_add(u) / 2),
            };

            if let Some(median_nanos) = median_nanos {
                site_rtt.push(crate::lts2_sys::shared_types::SiteRtt {
                    timestamp: now,
                    site_hash,
                    median_rtt: (median_nanos as f64 / 1_000_000.0) as f32,
                });
            }
        });
        if !site_throughput.is_empty() {
            if crate::lts2_sys::site_throughput(&site_throughput).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }
        if !site_retransmits.is_empty() {
            if crate::lts2_sys::site_retransmits(&site_retransmits).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }
        if !site_rtt.is_empty() {
            if crate::lts2_sys::site_rtt(&site_rtt).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }
        if !site_cake_drops.is_empty() {
            if crate::lts2_sys::site_cake_drops(&site_cake_drops).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }
        if !site_cake_marks.is_empty() {
            if crate::lts2_sys::site_cake_marks(&site_cake_marks).is_err() {
                warn!("Error sending message to LTS2.");
            }
        }

        // Shaper utilization
        if counter % 60 == 0 {
            let (tx, rx) = tokio::sync::oneshot::channel();
            if system_usage_actor.send(tx).is_ok() {
                if let Ok(reply) = rx.blocking_recv() {
                    let avg_cpu =
                        reply.cpu_usage.iter().sum::<u32>() as f32 / reply.cpu_usage.len() as f32;
                    let peak_cpu: u32 = reply.cpu_usage.iter().copied().sum();
                    let memory = reply.ram_used as f32 / reply.total_ram as f32;

                    if let Err(e) =
                        crate::lts2_sys::shaper_utilization(now, avg_cpu, peak_cpu as f32, memory)
                    {
                        warn!("Error sending message to LTS2: {e:?}");
                    }
                }
            }
        }
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

/// Calculates a hash of the combination of network.json and shaped devices.
/// This uses the NON-INSIGHT version, always - this is deliberate. If Insight
/// is updating, but integrations are changing the original, we want to use
/// the original.
fn combined_devices_network_hash() -> anyhow::Result<i64> {
    let cfg = load_config()?;
    let nj_path = Path::new(&cfg.lqos_directory).join("network.json");
    let sd_path = Path::new(&cfg.lqos_directory).join("ShapedDevices.csv");

    let nj_as_string = read_to_string(nj_path)?;
    let sd_as_string = read_to_string(sd_path)?;
    let combined = format!("{}\n{}", nj_as_string, sd_as_string);
    let hash = hash_to_i64(&combined);

    Ok(hash)
}

fn lts2_needs_shaped_devices() -> bool {
    let stored_hash = LTS2_HASH.load(std::sync::atomic::Ordering::Relaxed);
    let new_hash = combined_devices_network_hash().unwrap_or(-1);
    tracing::debug!("Stored Hash: {}, New Hash: {}", stored_hash, new_hash);
    LTS2_HASH.store(new_hash, std::sync::atomic::Ordering::Relaxed);
    stored_hash != new_hash
}

static LTS2_HASH: AtomicI64 = AtomicI64::new(0);

/// Loads the local-only (not Insight) Shaped Devices for
/// transmission to Insight
fn load_local_shaped_devices() -> anyhow::Result<Vec<ShapedDevice>> {
    let cfg = load_config()?;
    let sd_path = Path::new(&cfg.lqos_directory).join("ShapedDevices.csv");
    if !sd_path.exists() {
        anyhow::bail!("ShapedDevices.csv does not exist");
    }
    let mut reader = ReaderBuilder::new()
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_path(sd_path)?;
    let mut devices = Vec::new(); // Note that this used to be supported_customers, but we're going to let it grow organically

    for result in reader.records() {
        if let Ok(result) = result {
            let device = ShapedDevice::from_csv(&result)?;
            devices.push(device);
        } else {
            anyhow::bail!("Error reading ShapedDevices.csv");
        }
    }
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_for_submission_small_rates() {
        // Small fractional rates should round up to 1
        assert_eq!(rate_for_submission(0.1), 1);
        assert_eq!(rate_for_submission(0.5), 1);
        assert_eq!(rate_for_submission(0.9), 1);
    }

    #[test]
    fn test_rate_for_submission_edge_case_one() {
        // Exactly 1.0 should stay 1
        assert_eq!(rate_for_submission(1.0), 1);
    }

    #[test]
    fn test_rate_for_submission_normal_rates() {
        // Normal rates should round to nearest integer
        assert_eq!(rate_for_submission(1.1), 1);
        assert_eq!(rate_for_submission(1.4), 1);
        assert_eq!(rate_for_submission(1.5), 2);
        assert_eq!(rate_for_submission(1.6), 2);
        assert_eq!(rate_for_submission(2.3), 2);
        assert_eq!(rate_for_submission(2.7), 3);
    }

    #[test]
    fn test_rate_for_submission_large_rates() {
        // Large rates should round normally
        assert_eq!(rate_for_submission(100.4), 100);
        assert_eq!(rate_for_submission(100.5), 101);
        assert_eq!(rate_for_submission(1000.2), 1000);
        assert_eq!(rate_for_submission(1000.8), 1001);
    }

    #[test]
    fn test_rate_for_submission_prevents_zero() {
        // Should never return 0, even for very small inputs
        assert_eq!(rate_for_submission(0.01), 1);
        assert_eq!(rate_for_submission(0.001), 1);
    }

    #[test]
    fn test_rate_for_submission_preserves_integers() {
        // Integer inputs should be preserved exactly
        assert_eq!(rate_for_submission(5.0), 5);
        assert_eq!(rate_for_submission(10.0), 10);
        assert_eq!(rate_for_submission(100.0), 100);
    }
}
