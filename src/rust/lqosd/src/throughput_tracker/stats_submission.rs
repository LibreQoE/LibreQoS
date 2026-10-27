#![allow(dead_code)] // K3-B carve-out: LTS2/Insight stats payload builders are retained for 3a/GA; the kernel feed that invoked them is retired.
use crate::lts2_sys::shared_types::{CircuitCakeDrops, CircuitCakeMarks};
use crate::system_stats::SystemStats;
use crate::throughput_tracker::flow_data::{FlowbeeEffectiveDirection, live_active_flow_count};
use crate::throughput_tracker::throughput_entry::ThroughputEntry;
use crate::throughput_tracker::{
    CIRCUIT_RTT_BUFFERS, Lts2Circuit, Lts2Device, RawNetJs, THROUGHPUT_TRACKER, min_max_median_rtt,
    min_max_median_tcp_retransmits, resolve_flow_device,
};
use fxhash::{FxHashMap, FxHashSet};
use lqos_config::{ShapedDevice, TopologyCanonicalStateFile, load_config};
use lqos_queue_tracker::queue_stats_stale;
use lqos_queue_tracker::{ALL_QUEUE_SUMMARY, TOTAL_QUEUE_STATS};
use lqos_topology_compile::TopologyImportFile;
use lqos_utils::XdpIpAddress;
use lqos_utils::hash_to_i64;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::unix_now;
use std::io::Write;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::sync::atomic::AtomicI64;
use tracing::debug;
use tracing::info;
use tracing::log::warn;
use uuid::Uuid;

fn scale_u64_by_f64(value: u64, scale: f64) -> u64 {
    (value as f64 * scale) as u64
}

fn insight_topology_debug_path(config: &lqos_config::Config) -> PathBuf {
    config.debug_state_file_path("network.insight.debug.json")
}

fn write_insight_topology_debug_snapshot(
    config: &lqos_config::Config,
    raw_json: &str,
) -> anyhow::Result<()> {
    let path = insight_topology_debug_path(config);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension("tmp");
    let mut file = std::fs::File::create(&temp_path)?;
    file.write_all(raw_json.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    std::fs::rename(temp_path, path)?;
    Ok(())
}

fn load_insight_logical_network_json() -> anyhow::Result<String> {
    // Phase D: READ-ONLY. The debug snapshot is written at the explicit
    // submission point (`submit_topology_to_lts_if_changed`), never here.
    let config = load_config()?;
    let canonical = TopologyCanonicalStateFile::load_with_legacy_fallback(config.as_ref())?;
    Ok(serde_json::to_string_pretty(
        &canonical.insight_topology_network_json(),
    )?)
}

/// Temporary conversion function for LTS/Insight compatibility
fn rate_for_submission(rate_mbps: f32) -> u32 {
    if rate_mbps.is_nan() {
        warn!("LTS2 bandwidth value {rate_mbps} is not finite; clamping to 1 Mbps");
        return 1;
    }
    if rate_mbps.is_infinite() {
        if rate_mbps.is_sign_positive() {
            warn!("LTS2 bandwidth value {rate_mbps} exceeds u32::MAX; clamping to u32::MAX");
            return u32::MAX;
        }
        warn!("LTS2 bandwidth value {rate_mbps} is not finite; clamping to 1 Mbps");
        return 1;
    }

    if rate_mbps < 1.0 {
        1 // Round up small fractional rates to 1 Mbps for now
    } else if rate_mbps >= u32::MAX as f32 {
        warn!("LTS2 bandwidth value {rate_mbps} exceeds u32::MAX; clamping to u32::MAX");
        u32::MAX
    } else {
        rate_mbps.round() as u32 // Round to nearest integer
    }
}

fn clamp_lts2_u64_to_u32(field: &str, value: u64) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => {
            warn!("LTS2 {field} value {value} exceeds u32::MAX; clamping to u32::MAX");
            u32::MAX
        }
    }
}

fn clamp_lts2_u64_to_i32(field: &str, value: u64) -> i32 {
    match i32::try_from(value) {
        Ok(value) => value,
        Err(_) => {
            warn!("LTS2 {field} value {value} exceeds i32::MAX; clamping to i32::MAX");
            i32::MAX
        }
    }
}

/// `lqos-74t.56`: the runtime `network_json` store the trace reads carries
/// LOWERCASED node kinds (`lqos_network_devices` `dataplane_tree.rs`
/// `to_ascii_lowercase()` ⇒ `"site"`/`"ap"`/`"circuit"`), while the trace once
/// compared `== Some("Site")`. A real site therefore fell to the non-site arm
/// and `view_nonzero_sites` read 0 — a false "sites absent" reading that made
/// the earlier A1/A2 exoneration untrustworthy. Case-insensitive classify.
/// Diagnostic helper ONLY (gates the trace counters, never the emit). Removal
/// gate 2026-12-31, DRIVER-owned, together with `SITE-RETRANSMIT-TRACE`.
fn is_site_kind(node_type: Option<&str>) -> bool {
    node_type.is_some_and(|kind| kind.eq_ignore_ascii_case("site"))
}

fn lts2_rtt_millis_from_nanos(field: &str, nanos: u64) -> f32 {
    let millis = nanos as f64 / 1_000_000.0;
    if !millis.is_finite() || millis > f64::from(f32::MAX) {
        warn!("LTS2 {field} RTT value {nanos} ns exceeds f32::MAX ms; clamping to f32::MAX");
        f32::MAX
    } else {
        millis as f32
    }
}

fn queue_metrics_available(config: &lqos_config::Config) -> bool {
    !config.queues.queue_mode.is_observe() && !queue_stats_stale()
}

fn resolved_circuit_hash_for_submission(
    catalog: &lqos_network_devices::NetworkDevicesCatalog,
    ip: &XdpIpAddress,
    entry: &ThroughputEntry,
) -> Option<i64> {
    entry.circuit_hash.or_else(|| {
        resolve_flow_device(catalog, ip, entry.device_hash, entry.circuit_hash)
            .map(|device| device.circuit_hash)
    })
}

/// 3b DEFECT 2: true when the SHAPED (post-shaping, delivered) rate
/// exceeds the link cap — the only possible cause is counter corruption, since
/// post-shaping is ≤ cap by construction. NEVER compare the pre-shaping ingress
/// `actual` to the cap: the bridge actual is the DEMAND and legitimately exceeds
/// it under load (guarding it suppressed every loaded LTS submission). PURE.
fn bridge_spike_guard(
    shaped_down_bytes_per_second: u64,
    shaped_up_bytes_per_second: u64,
    downlink_mbps: u64,
    uplink_mbps: u64,
) -> bool {
    shaped_down_bytes_per_second.saturating_mul(8) > downlink_mbps.saturating_mul(1_000_000)
        || shaped_up_bytes_per_second.saturating_mul(8) > uplink_mbps.saturating_mul(1_000_000)
}

/// 3a LTS repoint: the bridge-source submission. Global + per-circuit from the
/// analytics snapshot, per-site from the tree live-view, CAKE from the queue
/// summary (bridge-fed since K2 slice 2). The kernel-only per-protocol packet
/// splits and the kernel network.json live writes are honest-empty on a bridge
/// box (zero, disclosed) — never fabricated.
fn submit_bridge_throughput_stats(
    config: &lqos_config::Config,
    scale: f64,
    counter: u8,
    system_usage_actor: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) -> bool {
    let queue_metrics_available = queue_metrics_available(config);
    let Ok(now) = unix_now() else {
        return false;
    };
    let snapshot = crate::bridge_analytics::snapshot();
    let shaped = snapshot.bridge_shaped_bytes_per_second;

 // 3b DEFECT 2: guard the SHAPED (delivered) rate against the
    // link cap. The bridge `actual` (pre-shaping ingress DEMAND) legitimately
    // exceeds the cap under load; guarding it suppressed every loaded
    // submission.
    // The root shaper bandwidth is the control plane's SINGLE SOURCE
    // (`Config::bandwidth_mbps()`: the v3 `[dataplane]` preferred, the legacy
    // `[queues]` fallback during the transition).
    // The guard + the instrument are integer-Mbps; the exact (possibly decimal)
    // rate lives in the config.
    let (down_cap_mbps, up_cap_mbps) = {
        let (down, up) = config.bandwidth_mbps();
        (down.round() as u64, up.round() as u64)
    };
    let spike = bridge_spike_guard(shaped.down, shaped.up, down_cap_mbps, up_cap_mbps);
    // `lqos-74t.2.1` instrument (removal gate 2026-12-31, DRIVER-owned): the
    // spike-guard INPUTS at the bridge submission decision (the guard is the
    // only early return in this function; a spike suppresses the flush).
    info!(
        "LTS bridge submission decision: shaped_down_bps={} shaped_up_bps={} downlink_cap_bps={} uplink_cap_bps={} (caps {} / {} Mbps) spike={spike}",
        shaped.down,
        shaped.up,
        down_cap_mbps.saturating_mul(1_000_000),
        up_cap_mbps.saturating_mul(1_000_000),
        down_cap_mbps,
        up_cap_mbps,
    );
    if spike {
        debug!("Spike detected - not submitting LTS");
        return false;
    }

 // 3b: push the topology on startup/topology change (the bridge
    // path previously returned before the kernel-only topology block, so the
    // ingestor never registered the shaper — 0 time-series rows).
    submit_topology_to_lts_if_changed(now);

    let packets = crate::bridge_analytics::total_enqueued_packets_per_second(&snapshot.by_ip);
    let (packets_down, packets_up) = (packets.down, packets.up);
    let (min_rtt, max_rtt, median_rtt) = match min_max_median_rtt() {
        Some(rtt) => (Some(rtt.min), Some(rtt.max), Some(rtt.median)),
        None => (None, None, None),
    };
    let tcp_retransmits = min_max_median_tcp_retransmits();
 // 3b display semantics: customer-facing throughput = the
    // DELIVERED (shaped) rate — kernel parity, where the headline was the
    // observed/delivered rate. The bridge ingress DEMAND is a separate labeled
    // local metric and is not the LTS headline.
    if let Err(e) = crate::lts2_sys::total_throughput(crate::lts2_sys::shared_types::ShaperThroughput {
        tick: now,
        bytes_per_second_down: scale_u64_by_f64(shaped.down, scale) as i64,
        bytes_per_second_up: scale_u64_by_f64(shaped.up, scale) as i64,
        shaped_bytes_per_second_down: scale_u64_by_f64(shaped.down, scale) as i64,
        shaped_bytes_per_second_up: scale_u64_by_f64(shaped.up, scale) as i64,
        packets_down: scale_u64_by_f64(packets_down, scale) as i64,
        packets_up: scale_u64_by_f64(packets_up, scale) as i64,
        // lqos-74t.2.4 (Root cause B): the shaper denominator is the BASE-EXACT
        // by_ip TCP-packet sum — the SAME source + down-only convention as the
        // numerator (`tcp_retransmits.down`/`.up`, from `by_ip[*].tcp_retransmits`).
        // `min_max_median_tcp_retransmits()` sums `by_ip[*].tcp_packets` into
        // `tcp_down`/`tcp_up` in that same loop
        // (throughput_tracker/mod.rs:1340-1343), so retransmits/tcp_packets is a
        // same-base ratio.  (The kernel `THROUGHPUT_TRACKER.*` stores are never
        // written on a bridge box; the per-FLOW
        // `snapshot.protocol_packets_per_second.tcp` split is a DIFFERENT
        // source/convention and MUST NOT be mixed in here — it would skew the %.)
        tcp_packets_down: scale_u64_by_f64(tcp_retransmits.tcp_down, scale) as i64,
        tcp_packets_up: scale_u64_by_f64(tcp_retransmits.tcp_up, scale) as i64,
        udp_packets_down: 0,
        udp_packets_up: 0,
        icmp_packets_down: 0,
        icmp_packets_up: 0,
        max_rtt,
        min_rtt,
        median_rtt,
        tcp_retransmits_down: tcp_retransmits.down,
        tcp_retransmits_up: tcp_retransmits.up,
        cake_marks_down: if queue_metrics_available {
            clamp_lts2_u64_to_i32("total cake_marks_down", TOTAL_QUEUE_STATS.marks.get_down())
        } else {
            0
        },
        cake_marks_up: if queue_metrics_available {
            clamp_lts2_u64_to_i32("total cake_marks_up", TOTAL_QUEUE_STATS.marks.get_up())
        } else {
            0
        },
        cake_drops_down: if queue_metrics_available {
            clamp_lts2_u64_to_i32("total cake_drops_down", TOTAL_QUEUE_STATS.drops.get_down())
        } else {
            0
        },
        cake_drops_up: if queue_metrics_available {
            clamp_lts2_u64_to_i32("total cake_drops_up", TOTAL_QUEUE_STATS.drops.get_up())
        } else {
            0
        },
    }) {
        warn!("Error sending message to LTS2. {e:?}");
    }
    if let Err(e) = crate::lts2_sys::flow_count(now, snapshot.active_flow_ips.len() as u64) {
        debug!("Error sending message to LTS2. {e:?}");
    }

    // Per-circuit throughput / retransmits / RTT.
    let catalog = lqos_network_devices::network_devices_catalog();
    const CRAZY_LIMIT: u64 = 8;
    let mut plan_lookup: FxHashMap<i64, (u64, u64)> = FxHashMap::default();
    for device in catalog.iter_all_devices() {
        let entry = plan_lookup.entry(device.circuit_hash).or_insert((0, 0));
        entry.0 = entry
            .0
            .max(device.download_max_mbps.round() as u64 * 1_000_000 * CRAZY_LIMIT);
        entry.1 = entry
            .1
            .max(device.upload_max_mbps.round() as u64 * 1_000_000 * CRAZY_LIMIT);
    }

    let mut circuit_throughput: Vec<crate::lts2_sys::shared_types::CircuitThroughput> = Vec::new();
    let mut circuit_retransmits: Vec<crate::lts2_sys::shared_types::CircuitRetransmits> = Vec::new();
    let mut circuit_rtt: Vec<crate::lts2_sys::shared_types::CircuitRtt> = Vec::new();
    for (circuit_id, row) in snapshot.by_circuit_id.iter() {
        let circuit_hash = hash_to_i64(circuit_id);
 // 3b display semantics: customer-facing per-circuit
        // throughput = the DELIVERED (shaped) series; `CircuitLiveRow.
        // bytes_per_second` is the enqueued demand.
        if let Some((dl, ul)) = plan_lookup.get(&circuit_hash)
            && (row.shaped_bytes_per_second.down > *dl || row.shaped_bytes_per_second.up > *ul)
        {
            continue; // crazy values: skip, matching the kernel guard
        }
        let sample = row.tcp_retransmit_sample;
        circuit_throughput.push(crate::lts2_sys::shared_types::CircuitThroughput {
            timestamp: now,
            circuit_hash,
            download_bytes: scale_u64_by_f64(row.shaped_bytes_per_second.down, scale),
            upload_bytes: scale_u64_by_f64(row.shaped_bytes_per_second.up, scale),
            packets_down: 0,
            packets_up: 0,
            tcp_packets_down: scale_u64_by_f64(sample.down.packets.get(), scale),
            tcp_packets_up: scale_u64_by_f64(sample.up.packets.get(), scale),
            udp_packets_down: 0,
            udp_packets_up: 0,
            icmp_packets_down: 0,
            icmp_packets_up: 0,
        });
        let retransmits_down = sample.down.retransmits.get();
        let retransmits_up = sample.up.retransmits.get();
        if retransmits_down != 0 || retransmits_up != 0 {
            circuit_retransmits.push(crate::lts2_sys::shared_types::CircuitRetransmits {
                timestamp: now,
                circuit_hash,
                tcp_retransmits_down: clamp_lts2_u64_to_u32(
                    "circuit tcp_retransmits_down",
                    retransmits_down,
                ),
                tcp_retransmits_up: clamp_lts2_u64_to_u32(
                    "circuit tcp_retransmits_up",
                    retransmits_up,
                ),
            });
        }
        let down_nanos = row.rtt_current_p50_nanos.down.unwrap_or(0);
        let up_nanos = row.rtt_current_p50_nanos.up.unwrap_or(0);
        let median_nanos = match (down_nanos, up_nanos) {
            (0, 0) => None,
            (down, 0) => Some(down),
            (0, up) => Some(up),
            (down, up) => Some(down.saturating_add(up) / 2),
        };
        if let Some(median_nanos) = median_nanos {
            circuit_rtt.push(crate::lts2_sys::shared_types::CircuitRtt {
                timestamp: now,
                circuit_hash,
                median_rtt: lts2_rtt_millis_from_nanos("circuit median", median_nanos),
            });
        }
    }
    if !circuit_throughput.is_empty()
        && crate::lts2_sys::circuit_throughput(&circuit_throughput).is_err()
    {
        warn!("Error sending message to LTS2.");
    }
    if !circuit_retransmits.is_empty()
        && crate::lts2_sys::circuit_retransmits(&circuit_retransmits).is_err()
    {
        warn!("Error sending message to LTS2.");
    }
    if !circuit_rtt.is_empty() && crate::lts2_sys::circuit_rtt(&circuit_rtt).is_err() {
        warn!("Error sending message to LTS2.");
    }

    // Per-circuit CAKE (the queue summary is bridge-fed; same shape as kernel).
    let mut cake_drops: Vec<CircuitCakeDrops> = Vec::new();
    let mut cake_marks: Vec<CircuitCakeMarks> = Vec::new();
    if queue_metrics_available {
        ALL_QUEUE_SUMMARY.iterate_queues(|circuit_hash, drops, marks| {
            if drops.not_zero() {
                cake_drops.push(CircuitCakeDrops {
                    timestamp: now,
                    circuit_hash,
                    cake_drops_down: clamp_lts2_u64_to_u32(
                        "circuit cake_drops_down",
                        drops.get_down(),
                    ),
                    cake_drops_up: clamp_lts2_u64_to_u32("circuit cake_drops_up", drops.get_up()),
                });
            }
            if marks.not_zero() {
                cake_marks.push(CircuitCakeMarks {
                    timestamp: now,
                    circuit_hash,
                    cake_marks_down: clamp_lts2_u64_to_u32(
                        "circuit cake_marks_down",
                        marks.get_down(),
                    ),
                    cake_marks_up: clamp_lts2_u64_to_u32("circuit cake_marks_up", marks.get_up()),
                });
            }
        });
    }
    if !cake_drops.is_empty() && crate::lts2_sys::circuit_cake_drops(&cake_drops).is_err() {
        warn!("Error sending message to LTS2.");
    }
    if !cake_marks.is_empty() && crate::lts2_sys::circuit_cake_marks(&cake_marks).is_err() {
        warn!("Error sending message to LTS2.");
    }

    // Per-site throughput / retransmits / RTT from the tree live-view.
    if let Some(view) = crate::bridge_analytics::tree_live_view() {
        let mut site_throughput: Vec<crate::lts2_sys::shared_types::SiteThroughput> = Vec::new();
        let mut site_retransmits: Vec<crate::lts2_sys::shared_types::SiteRetransmits> = Vec::new();
        let mut site_rtt: Vec<crate::lts2_sys::shared_types::SiteRtt> = Vec::new();
        // `lqos-74t.56` instrument (removal gate 2026-12-31, DRIVER-owned): the
        // SiteRetransmits gap. One cold-path line per collation. It proves
        // whether a non-zero SITE idx exists in the live view (not a sample),
        // emits the emitted `site_hash` set AND the submitted `network_tree`
        // site-hash set for a direct diff, and splits FORK-A into the
        // unmatched-circuit count vs "resolved but no Site in `parents`".
        let mut trace_emitted: Vec<String> = Vec::new();
        let mut view_nonzero_sites: usize = 0;
        let mut view_nonzero_nonsites: usize = 0;
        let mut view_nonzero_no_site_parents: usize = 0;
        lqos_network_devices::with_network_json_read(|net_json| {
            let nodes = net_json.get_nodes_when_ready();
            for (idx, node) in nodes.iter().enumerate() {
                let site_hash = hash_to_i64(&node.name);
                if let Some((down, up)) = view.current_retransmits.get(&idx)
                    && (*down != 0 || *up != 0)
                {
                    if is_site_kind(node.node_type.as_deref()) {
                        view_nonzero_sites += 1;
                    } else {
                        view_nonzero_nonsites += 1;
                        let has_site_ancestor = node.parents.iter().any(|p| {
                            is_site_kind(nodes.get(*p).and_then(|a| a.node_type.as_deref()))
                        });
                        if !has_site_ancestor {
                            view_nonzero_no_site_parents += 1;
                        }
                    }
                }
                if let Some(tp) = view.current_throughput.get(&idx)
                    && (tp.down != 0 || tp.up != 0)
                {
                    site_throughput.push(crate::lts2_sys::shared_types::SiteThroughput {
                        timestamp: now,
                        site_hash,
                        download_bytes: scale_u64_by_f64(tp.down, scale),
                        upload_bytes: scale_u64_by_f64(tp.up, scale),
                        packets_down: 0,
                        packets_up: 0,
                        // lqos-74t.2.4 (Root cause B): the per-node TCP packet
                        // counts the tree live-view carries —
                        // `current_tcp_retransmit_packets`, written in the tree
                        // roll-up (bridge_analytics/mod.rs:1903-1905:
                        // `packets.i += row.tcp_retransmit_sample.<dir>.packets`),
                        // from the SAME loop that writes the numerator's
                        // `current_retransmits` ⇒ a base-exact site ratio.  (v2's
                        // `network_json.rs::add_retransmit_cycle` is DEAD in v3 —
                        // no production caller.)  0 only when this node has no
                        // view entry.
                        packets_tcp_down: view
                            .current_tcp_retransmit_packets
                            .get(&idx)
                            .map(|(down, _up)| scale_u64_by_f64(*down, scale))
                            .unwrap_or(0),
                        packets_tcp_up: view
                            .current_tcp_retransmit_packets
                            .get(&idx)
                            .map(|(_down, up)| scale_u64_by_f64(*up, scale))
                            .unwrap_or(0),
                        packets_udp_down: 0,
                        packets_udp_up: 0,
                        packets_icmp_down: 0,
                        packets_icmp_up: 0,
                    });
                }
                if let Some((down, up)) = view.current_retransmits.get(&idx)
                    && (*down != 0 || *up != 0)
                {
                    if trace_emitted.len() < 25 {
                        trace_emitted.push(format!(
                            "{}|hash={}|type={}|{}/{}",
                            node.name,
                            site_hash,
                            node.node_type.as_deref().unwrap_or("?"),
                            down,
                            up
                        ));
                    }
                    site_retransmits.push(crate::lts2_sys::shared_types::SiteRetransmits {
                        timestamp: now,
                        site_hash,
                        tcp_retransmits_down: clamp_lts2_u64_to_u32(
                            "site tcp_retransmits_down",
                            *down,
                        ),
                        tcp_retransmits_up: clamp_lts2_u64_to_u32(
                            "site tcp_retransmits_up",
                            *up,
                        ),
                    });
                }
                if let Some(rtts) = view.rtts.get(&idx)
                    && let Some(median) = median_f32(rtts)
                {
                    site_rtt.push(crate::lts2_sys::shared_types::SiteRtt {
                        timestamp: now,
                        site_hash,
                        median_rtt: median,
                    });
                }
            }
        });
        // The submitted `network_tree` site identity (`Lts2NetJs`) — the key the
        // emitted `site_hash` must match (`to_lts2`, throughput_tracker/mod.rs).
        let mut tree_sites: Vec<String> = Vec::new();
        if let Ok(raw) = load_insight_logical_network_json()
            && let Ok(json) = serde_json::from_str::<RawNetJs>(&raw)
        {
            fn collect_sites(nodes: &[super::Lts2NetJs], out: &mut Vec<String>) {
                for n in nodes {
                    if n.site_type.as_deref() == Some("Site") && out.len() < 25 {
                        out.push(format!("{}|hash={}", n.name, n.site_hash));
                    }
                    collect_sites(&n.children, out);
                }
            }
            let flat: Vec<super::Lts2NetJs> = json.iter().map(|(k, v)| v.to_lts2(k)).collect();
            collect_sites(&flat, &mut tree_sites);
        }
        let source_nonzero_circuits = snapshot
            .by_circuit_id
            .values()
            .filter(|r| {
                r.tcp_retransmit_sample.down.retransmits.get() != 0
                    || r.tcp_retransmit_sample.up.retransmits.get() != 0
            })
            .count();
        info!(
            "SITE-RETRANSMIT-TRACE: circuit_batch={} site_batch={} view_idxs={} view_nonzero_sites={} view_nonzero_nonsites={} view_nonzero_no_site_parents={} unmatched_circuits={} source_nonzero_circuits={} emitted=[{}] tree_sites=[{}]",
            circuit_retransmits.len(),
            site_retransmits.len(),
            view.current_retransmits.len(),
            view_nonzero_sites,
            view_nonzero_nonsites,
            view_nonzero_no_site_parents,
            view.unmatched_circuits,
            source_nonzero_circuits,
            trace_emitted.join(", "),
            tree_sites.join(", "),
        );
        if !site_throughput.is_empty()
            && crate::lts2_sys::site_throughput(&site_throughput).is_err()
        {
            warn!("Error sending message to LTS2.");
        }
        if !site_retransmits.is_empty()
            && crate::lts2_sys::site_retransmits(&site_retransmits).is_err()
        {
            warn!("Error sending message to LTS2.");
        }
        if !site_rtt.is_empty() && crate::lts2_sys::site_rtt(&site_rtt).is_err() {
            warn!("Error sending message to LTS2.");
        }
    }

    // Shaper utilization (unchanged cadence/threshold).
    if counter.is_multiple_of(60) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if system_usage_actor.send(tx).is_ok()
            && let Ok(reply) = rx.blocking_recv()
        {
            let avg_cpu = reply.cpu_usage.iter().sum::<u32>() as f32 / reply.cpu_usage.len() as f32;
            let peak_cpu: u32 = reply.cpu_usage.iter().copied().sum();
            let memory = reply.ram_used as f32 / reply.total_ram as f32;
            if let Err(e) = crate::lts2_sys::shaper_utilization(now, avg_cpu, peak_cpu as f32, memory)
            {
                warn!("Error sending message to LTS2: {e:?}");
            }
        }
    }
    true
}

/// 3b pure flush gate: flush ONLY after a completed submission
/// batch — the config/license gates and the spike guard return before it, so a
/// skipped submission never flushes. PURE.
fn lts_flush_after(submitted: bool) -> bool {
    submitted
}

/// 3b: the ingest flush. `ingest_batch_complete` is the ONLY path
/// that turns the filled batch queue into `SubmitChunks`; K3-A deleted its
/// caller, so a bridge shaper filled the queue forever and the ingestor saw
/// nothing. Side effects: enqueues the batch-complete command.
fn flush_lts_batch() {
    if let Err(e) = crate::lts2_sys::ingest_batch_complete() {
        tracing::log::warn!("Error sending message to LTS2: {e:?}");
    }
}

fn median_f32(values: &[f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let midpoint = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        Some(sorted[midpoint])
    } else {
        Some((sorted[midpoint - 1] + sorted[midpoint]) / 2.0)
    }
}

pub(crate) fn submit_throughput_stats(
    scale: f64,
    counter: u8,
    system_usage_actor: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) {
    // Load the config
    let Ok(config) = load_config() else {
        info!("LTS submit decision: config load FAILED (no submission)");
        return;
    };
    let queue_metrics_available = queue_metrics_available(config.as_ref());

    // `lqos-74t.2.1` instrument (removal gate 2026-12-31, DRIVER-owned): the
    // submission-decision inputs, logged BEFORE any early return so a
    // never-submitting box is visible (the per-session instrument never fires
    // if the trigger/submit path bails here). Cold path: one call per collation
    // window (default 60 s, and only when the trigger fires at all).
    info!(
        "LTS submit decision: gather_stats={} bridge_analytics_active={} can_submit_long_term_stats={} license_key_present={}",
        config.long_term_stats.gather_stats,
        crate::bridge_analytics::is_active(),
        crate::lts2_sys::can_submit_long_term_stats(),
        config
            .long_term_stats
            .license_key
            .as_ref()
            .is_some_and(|k| !k.trim().is_empty()),
    );

    // Bail out if we don't have gather stats or a license key
    if !config.long_term_stats.gather_stats {
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

    if !crate::lts2_sys::can_submit_long_term_stats() {
        return;
    }

    // 3a LTS repoint: on a bridge box every payload comes from the actor's
    // snapshot + the tree live-view (the kernel stores are never written).
    if crate::bridge_analytics::is_active() {
 // 3b: flush the ingest batch after a completed submission
        // (the K3-A deletion orphaned `ingest_batch_complete()`; without it the
        // queue fills forever and the ingestor never receives anything). The
        // flush is skipped exactly when the bridge submission bails (the
        // config/license gates or the spike guard).
        if lts_flush_after(submit_bridge_throughput_stats(
            &config,
            scale,
            counter,
            system_usage_actor,
        )) {
            flush_lts_batch();
        }
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
    let bits_per_second = THROUGHPUT_TRACKER.actual_bits_per_second();

    // Check that the stats haven't gone wonky and don't submit obviously bad data.
    let (down_cap_mbps, up_cap_mbps) = {
        let (down, up) = config.bandwidth_mbps();
        (down.round() as u64, up.round() as u64)
    };
    if bits_per_second.down > (down_cap_mbps * 1_000_000) {
        debug!("Spike detected - not submitting LTS");
        return; // Do not submit these stats
    }
    if bits_per_second.up > (up_cap_mbps * 1_000_000) {
        debug!("Spike detected - not submitting LTS");
        return; // Do not submit these stats
    }

    /////////////////////////////////////////////////////////////////
    // Insight Block
    if let Ok(now) = unix_now() {
 // 3b: the shared topology push (bridge + kernel).
        submit_topology_to_lts_if_changed(now);

        // Send top-level throughput stats to LTS2
        let bytes = THROUGHPUT_TRACKER.actual_bytes_per_second.as_down_up();
        let shaped_bytes = THROUGHPUT_TRACKER
            .shaped_actual_bytes_per_second
            .as_down_up();
        let mut min_rtt = None;
        let mut max_rtt = None;
        let mut median_rtt = None;
        if let Some(rtt_data) = min_max_median_rtt() {
            min_rtt = Some(rtt_data.min);
            max_rtt = Some(rtt_data.max);
            median_rtt = Some(rtt_data.median);
        }
        let tcp_retransmits = min_max_median_tcp_retransmits();
        if crate::lts2_sys::total_throughput(crate::lts2_sys::shared_types::ShaperThroughput {
            tick: now,
            bytes_per_second_down: scale_u64_by_f64(bytes.down, scale) as i64,
            bytes_per_second_up: scale_u64_by_f64(bytes.up, scale) as i64,
            shaped_bytes_per_second_down: scale_u64_by_f64(shaped_bytes.down, scale) as i64,
            shaped_bytes_per_second_up: scale_u64_by_f64(shaped_bytes.up, scale) as i64,
            packets_down: scale_u64_by_f64(packets_per_second.0, scale) as i64,
            packets_up: scale_u64_by_f64(packets_per_second.1, scale) as i64,
            tcp_packets_down: scale_u64_by_f64(tcp_packets_per_second.0, scale) as i64,
            tcp_packets_up: scale_u64_by_f64(tcp_packets_per_second.1, scale) as i64,
            udp_packets_down: scale_u64_by_f64(udp_packets_per_second.0, scale) as i64,
            udp_packets_up: scale_u64_by_f64(udp_packets_per_second.1, scale) as i64,
            icmp_packets_down: scale_u64_by_f64(icmp_packets_per_second.0, scale) as i64,
            icmp_packets_up: scale_u64_by_f64(icmp_packets_per_second.1, scale) as i64,
            max_rtt,
            min_rtt,
            median_rtt,
            tcp_retransmits_down: tcp_retransmits.down,
            tcp_retransmits_up: tcp_retransmits.up,
            cake_marks_down: if queue_metrics_available {
                clamp_lts2_u64_to_i32("total cake_marks_down", TOTAL_QUEUE_STATS.marks.get_down())
            } else {
                0
            },
            cake_marks_up: if queue_metrics_available {
                clamp_lts2_u64_to_i32("total cake_marks_up", TOTAL_QUEUE_STATS.marks.get_up())
            } else {
                0
            },
            cake_drops_down: if queue_metrics_available {
                clamp_lts2_u64_to_i32("total cake_drops_down", TOTAL_QUEUE_STATS.drops.get_down())
            } else {
                0
            },
            cake_drops_up: if queue_metrics_available {
                clamp_lts2_u64_to_i32("total cake_drops_up", TOTAL_QUEUE_STATS.drops.get_up())
            } else {
                0
            },
        })
        .is_err()
        {
            warn!("Error sending message to LTS2.");
        }
        if let Err(e) = crate::lts2_sys::flow_count(now, live_active_flow_count()) {
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

        let catalog = lqos_network_devices::network_devices_catalog();
        const CRAZY_LIMIT: u64 = 8; // 8x the max bandwidth
        let mut plan_lookup: FxHashMap<i64, (u64, u64)> = FxHashMap::default();
        for device in catalog.iter_all_devices() {
            let entry = plan_lookup.entry(device.circuit_hash).or_insert((0, 0));
            entry.0 = entry
                .0
                .max(device.download_max_mbps.round() as u64 * 1_000_000 * CRAZY_LIMIT);
            entry.1 = entry
                .1
                .max(device.upload_max_mbps.round() as u64 * 1_000_000 * CRAZY_LIMIT);
        }

        THROUGHPUT_TRACKER
            .raw_data
            .lock()
            .iter()
            .filter_map(|(ip, h)| {
                let circuit_hash = resolved_circuit_hash_for_submission(&catalog, ip, h)?;
                h.actual_bytes_per_second
                    .not_zero()
                    .then_some((circuit_hash, h))
            })
            .for_each(|(circuit_hash, h)| {
                let mut crazy = false;
                if let Some((dl, ul)) = plan_lookup.get(&circuit_hash)
                    && (h.actual_bytes_per_second.down > *dl || h.actual_bytes_per_second.up > *ul)
                {
                    crazy_values.insert(circuit_hash);
                    crazy = true;
                }

                if crazy {
                    return;
                }
                if let Some(c) = circuit_throughput.get_mut(&circuit_hash) {
                    c.bytes += h.actual_bytes_per_second;
                    c.packets += h.packets_per_second;
                    c.tcp_packets += h.tcp_packets;
                    c.udp_packets += h.udp_packets;
                    c.icmp_packets += h.icmp_packets;
                } else {
                    circuit_throughput.insert(
                        circuit_hash,
                        CircuitThroughputTemp {
                            bytes: h.actual_bytes_per_second,
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
            .filter_map(|(ip, h)| {
                let circuit_hash = resolved_circuit_hash_for_submission(&catalog, ip, h)?;
                (h.tcp_retransmits.not_zero() && !crazy_values.contains(&circuit_hash))
                    .then_some((circuit_hash, h))
            })
            .for_each(|(circuit_hash, h)| {
                if let Some(c) = circuit_retransmits.get_mut(&circuit_hash) {
                    *c += h.tcp_retransmits;
                } else {
                    circuit_retransmits.insert(circuit_hash, h.tcp_retransmits);
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
                tcp_retransmits_down: clamp_lts2_u64_to_u32("circuit tcp_retransmits_down", v.down),
                tcp_retransmits_up: clamp_lts2_u64_to_u32("circuit tcp_retransmits_up", v.up),
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
                    median_rtt: lts2_rtt_millis_from_nanos("circuit median", median_nanos),
                })
            })
            .collect::<Vec<_>>();
        if crate::lts2_sys::circuit_rtt(&circuit_rtt_batch).is_err() {
            warn!("Error sending message to LTS2.");
        }

        // Per host CAKE stats
        let mut cake_drops: Vec<CircuitCakeDrops> = Vec::new();
        let mut cake_marks: Vec<CircuitCakeMarks> = Vec::new();
        if queue_metrics_available {
            ALL_QUEUE_SUMMARY.iterate_queues(|circuit_hash, drops, marks| {
                if drops.not_zero() {
                    cake_drops.push(CircuitCakeDrops {
                        timestamp: now,
                        circuit_hash,
                        cake_drops_down: clamp_lts2_u64_to_u32(
                            "circuit cake_drops_down",
                            drops.get_down(),
                        ),
                        cake_drops_up: clamp_lts2_u64_to_u32(
                            "circuit cake_drops_up",
                            drops.get_up(),
                        ),
                    });
                }
                if marks.not_zero() {
                    cake_marks.push(CircuitCakeMarks {
                        timestamp: now,
                        circuit_hash,
                        cake_marks_down: clamp_lts2_u64_to_u32(
                            "circuit cake_marks_down",
                            marks.get_down(),
                        ),
                        cake_marks_up: clamp_lts2_u64_to_u32(
                            "circuit cake_marks_up",
                            marks.get_up(),
                        ),
                    });
                }
            });
        }
        if !cake_drops.is_empty() && crate::lts2_sys::circuit_cake_drops(&cake_drops).is_err() {
            warn!("Error sending message to LTS2.");
        }
        if !cake_marks.is_empty() && crate::lts2_sys::circuit_cake_marks(&cake_marks).is_err() {
            warn!("Error sending message to LTS2.");
        }

        // Network tree stats
        let tree = lqos_network_devices::with_network_json_read(|net_json| {
            net_json.get_nodes_when_ready().clone()
        });
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
                    tcp_retransmits_down: clamp_lts2_u64_to_u32(
                        "site tcp_retransmits_down",
                        node.current_tcp_retransmits.down,
                    ),
                    tcp_retransmits_up: clamp_lts2_u64_to_u32(
                        "site tcp_retransmits_up",
                        node.current_tcp_retransmits.up,
                    ),
                });
            }
            if queue_metrics_available && node.current_drops.not_zero() {
                site_cake_drops.push(crate::lts2_sys::shared_types::SiteCakeDrops {
                    timestamp: now,
                    site_hash,
                    cake_drops_down: clamp_lts2_u64_to_u32(
                        "site cake_drops_down",
                        node.current_drops.get_down(),
                    ),
                    cake_drops_up: clamp_lts2_u64_to_u32(
                        "site cake_drops_up",
                        node.current_drops.get_up(),
                    ),
                });
            }
            if queue_metrics_available && node.current_marks.not_zero() {
                site_cake_marks.push(crate::lts2_sys::shared_types::SiteCakeMarks {
                    timestamp: now,
                    site_hash,
                    cake_marks_down: clamp_lts2_u64_to_u32(
                        "site cake_marks_down",
                        node.current_marks.get_down(),
                    ),
                    cake_marks_up: clamp_lts2_u64_to_u32(
                        "site cake_marks_up",
                        node.current_marks.get_up(),
                    ),
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
                    median_rtt: lts2_rtt_millis_from_nanos("site median", median_nanos),
                });
            }
        });
        if !site_throughput.is_empty()
            && crate::lts2_sys::site_throughput(&site_throughput).is_err()
        {
            warn!("Error sending message to LTS2.");
        }
        if !site_retransmits.is_empty()
            && crate::lts2_sys::site_retransmits(&site_retransmits).is_err()
        {
            warn!("Error sending message to LTS2.");
        }
        if !site_rtt.is_empty() && crate::lts2_sys::site_rtt(&site_rtt).is_err() {
            warn!("Error sending message to LTS2.");
        }
        if queue_metrics_available
            && !site_cake_drops.is_empty()
            && crate::lts2_sys::site_cake_drops(&site_cake_drops).is_err()
        {
            warn!("Error sending message to LTS2.");
        }
        if queue_metrics_available
            && !site_cake_marks.is_empty()
            && crate::lts2_sys::site_cake_marks(&site_cake_marks).is_err()
        {
            warn!("Error sending message to LTS2.");
        }

        // Shaper utilization
        if counter.is_multiple_of(60) {
            let (tx, rx) = tokio::sync::oneshot::channel();
            if system_usage_actor.send(tx).is_ok()
                && let Ok(reply) = rx.blocking_recv()
            {
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

fn ip4_to_bytes(ip: (Ipv4Addr, u32)) -> ([u8; 4], u8) {
    let bytes = ip.0.octets();
    (bytes, ip.1 as u8)
}

fn ip6_to_bytes(ip: (Ipv6Addr, u32)) -> ([u8; 16], u8) {
    let bytes = ip.0.octets();
    (bytes, ip.1 as u8)
}

/// Calculates a hash of the combination of the Insight logical topology tree and shaped devices.
///
/// This intentionally uses the Insight-only logical topology export rather than any
/// runtime-effective topology or local compatibility tree.
fn combined_devices_network_hash() -> anyhow::Result<i64> {
    let cfg = load_config()?;

    let nj_as_string = load_insight_logical_network_json()?;
    // Phase C S4: the compiled import is the ONE source.
    let topology_import = TopologyImportFile::load(cfg.as_ref())?
        .ok_or_else(|| anyhow::anyhow!("topology_import.json does not exist"))?;
    let sd_as_string = serde_json::to_string(
        &topology_import
            .into_imported_bundle()
            .shaped_devices
            .devices,
    )?;
    let dynamic_snapshot = lqos_network_devices::dynamic_circuits_snapshot();
    let mut dynamic_devices: Vec<ShapedDevice> = dynamic_snapshot
        .iter()
        .map(|circuit| circuit.shaped.clone())
        .collect();
    dynamic_devices.sort_by(|a, b| {
        (
            a.circuit_id.trim().to_ascii_lowercase(),
            a.device_id.trim().to_ascii_lowercase(),
        )
            .cmp(&(
                b.circuit_id.trim().to_ascii_lowercase(),
                b.device_id.trim().to_ascii_lowercase(),
            ))
    });
    let dynamic_as_string = serde_json::to_string(&dynamic_devices)?;

    let combined = format!("{}\n{}\n{}", nj_as_string, sd_as_string, dynamic_as_string);
    let hash = hash_to_i64(&combined);

    Ok(hash)
}

fn lts2_needs_shaped_devices() -> bool {
    let stored_hash = LTS2_HASH.load(std::sync::atomic::Ordering::Relaxed);
    let new_hash = combined_devices_network_hash().unwrap_or(-1);
    tracing::debug!("Stored Hash: {}, New Hash: {}", stored_hash, new_hash);
    LTS2_HASH.store(new_hash, std::sync::atomic::Ordering::Relaxed);
    topology_hash_changed(stored_hash, new_hash)
}

/// 3b pure gate: the topology push fires ONLY when the combined
/// devices/network hash changed — never a per-tick resend. PURE.
fn topology_hash_changed(stored_hash: i64, new_hash: i64) -> bool {
    stored_hash != new_hash
}

static LTS2_HASH: AtomicI64 = AtomicI64::new(0);

/// 3b: push the Insight topology (`network_tree`, `shaped_devices`,
/// and IP policies) when the combined devices/network hash changed. Shared by
/// the kernel AND bridge LTS paths: the bridge path used to return before the
/// kernel-only topology block, so a bridge shaper never registered with the
/// ingestor (0 time-series rows, empty Traffic Overview history) while the live
/// gateway still rendered. The hash gate makes this a no-op on unchanged
/// collation ticks. Returns whether it fired.
fn submit_topology_to_lts_if_changed(now: u64) -> bool {
    if !lts2_needs_shaped_devices() {
        return false;
    }
    tracing::info!("Sending topology to Insight");
    // Send the topology tree
    match load_insight_logical_network_json() {
        Err(err) => warn!("Unable to load Insight logical topology JSON. {err:?}"),
        Ok(raw_string) => {
            // Phase D: the DEBUG artifact is written HERE, at the explicit
            // "send topology to Insight" act — not from the load path.
            if let Ok(config) = load_config()
                && let Err(err) =
                    write_insight_topology_debug_snapshot(config.as_ref(), &raw_string)
            {
                warn!("Unable to write Insight topology debug snapshot. {err:?}");
            }
            match serde_json::from_str::<RawNetJs>(&raw_string) {
                Err(e) => warn!("Unable to parse Insight logical topology JSON. {e:?}"),
                Ok(json) => {
                    let lts2_format: Vec<_> = json.iter().map(|(k, v)| v.to_lts2(k)).collect();
                    if let Ok(bytes) = serde_cbor::to_vec(&lts2_format)
                        && let Err(e) = crate::lts2_sys::network_tree(now, &bytes)
                    {
                        warn!("Error sending message to Insight. {e:?}");
                    }
                }
            }
        }
    }

    // Send the shaped devices
    if let Ok(shaped_devices) = load_local_shaped_devices() {
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
                        download_min_mbps_exact: Some(device.download_min_mbps),
                        upload_min_mbps_exact: Some(device.upload_min_mbps),
                        download_max_mbps_exact: Some(device.download_max_mbps),
                        upload_max_mbps_exact: Some(device.upload_max_mbps),
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
        let devices_as_vec: Vec<Lts2Circuit> = circuit_map.into_values().collect();
        if let Ok(bytes) = serde_cbor::to_vec(&devices_as_vec)
            && crate::lts2_sys::shaped_devices(now, &bytes).is_err()
        {
            warn!("Error sending message to LTS2.");
        }
    }

    // Send permitted IP ranges at the same time. §`lqos-74t.31` W1: the
    // do-not-track subnets join the ignore set so hosted Insight honors them.
    if let Ok(config) = lqos_config::load_config() {
        let mut ignore = config.ip_ranges.ignore_subnets.clone();
        if let Some(flows) = config.flows.as_ref() {
            ignore.extend(flows.do_not_track_cidrs().iter().cloned());
        }
        if let Err(e) = crate::lts2_sys::ip_policies(&config.ip_ranges.allow_subnets, &ignore) {
            debug!("Error sending message to LTS2. {e:?}");
        }
    }
    true
}

/// Loads the local-only (not Insight) Shaped Devices for
/// transmission to Insight
fn load_local_shaped_devices() -> anyhow::Result<Vec<ShapedDevice>> {
    let cfg = load_config()?;
    let dynamic_snapshot = lqos_network_devices::dynamic_circuits_snapshot();
    // Phase C S4: the compiled import is the ONE source.
    let topology_import = TopologyImportFile::load(cfg.as_ref())?
        .ok_or_else(|| anyhow::anyhow!("topology_import.json does not exist"))?;
    let mut devices = topology_import
        .into_imported_bundle()
        .shaped_devices
        .devices;
    devices.extend(
        dynamic_snapshot
            .iter()
            .map(|circuit| circuit.shaped.clone()),
    );
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::throughput_tracker::throughput_entry::ThroughputEntry;
    use lqos_bus::TcHandle;
    use lqos_config::{ConfigShapedDevices, QueueMode, ShapedDevice};
    use lqos_network_devices::{NetworkDevicesCatalog, ShapedDevicesCatalog};
    use lqos_utils::XdpIpAddress;
    use lqos_utils::qoo::QoqScores;
    use lqos_utils::units::DownUpOrder;
    use std::net::Ipv4Addr;
    use std::sync::Arc;

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

    /// `lqos-74t.56` step-1: the trace's site classification must accept the
    /// runtime store's LOWERCASE kind (and any case), so a real site is counted
    /// as a site and never misrouted to the non-site arm.
    #[test]
    fn site_kind_classification_is_case_insensitive() {
        for kind in ["site", "Site", "SITE", "sItE"] {
            assert!(is_site_kind(Some(kind)), "{kind:?} must classify as a site");
        }
        for kind in ["ap", "AP", "circuit", "Circuit", "router", ""] {
            assert!(
                !is_site_kind(Some(kind)),
                "{kind:?} must NOT classify as a site"
            );
        }
        assert!(!is_site_kind(None), "a missing kind is not a site");
    }

    #[test]
    fn test_rate_for_submission_clamps_non_finite_and_huge_rates() {
        assert_eq!(rate_for_submission(f32::NAN), 1);
        assert_eq!(rate_for_submission(f32::INFINITY), u32::MAX);
        assert_eq!(rate_for_submission(f32::NEG_INFINITY), 1);
        assert_eq!(rate_for_submission(u32::MAX as f32), u32::MAX);
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

    #[test]
    fn lts2_u64_to_u32_clamps_protocol_limited_fields() {
        assert_eq!(clamp_lts2_u64_to_u32("test", u64::from(u32::MAX)), u32::MAX);
        assert_eq!(
            clamp_lts2_u64_to_u32("test", u64::from(u32::MAX) + 1),
            u32::MAX
        );
    }

    #[test]
    fn lts2_u64_to_i32_clamps_protocol_limited_fields() {
        assert_eq!(
            clamp_lts2_u64_to_i32("test", u64::try_from(i32::MAX).unwrap()),
            i32::MAX
        );
        assert_eq!(
            clamp_lts2_u64_to_i32("test", u64::try_from(i32::MAX).unwrap() + 1),
            i32::MAX
        );
    }

    #[test]
    fn lts2_rtt_millis_keeps_large_values_finite() {
        assert_eq!(lts2_rtt_millis_from_nanos("test", 1_500_000), 1.5);
        let millis = lts2_rtt_millis_from_nanos("test", u64::MAX);
        assert!(millis.is_finite());
        assert!(millis > 1.0e13);
    }

    #[test]
    fn queue_metrics_disabled_in_observe_mode() {
        let mut config = lqos_config::Config::default();
        config.queues.set_queue_mode(QueueMode::Observe);
        assert!(!queue_metrics_available(&config));
    }

    #[test]
    fn resolved_circuit_hash_falls_back_to_ip_lookup() {
        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(vec![ShapedDevice {
            circuit_id: "circuit-1".to_string(),
            circuit_name: "Circuit Alpha".to_string(),
            device_id: "device-1".to_string(),
            parent_node: "Parent-A".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 168, 1, 10), 32)],
            circuit_hash: hash_to_i64("circuit-1"),
            device_hash: hash_to_i64("device-1"),
            ..Default::default()
        }]);
        let shaped_catalog = ShapedDevicesCatalog::from_shaped_devices(Arc::new(shaped));
        let catalog = NetworkDevicesCatalog::from_snapshots(shaped_catalog, Arc::new(Vec::new()));
        let ip = XdpIpAddress::from_ip("192.168.1.10".parse().expect("test IP should parse"));
        let entry = ThroughputEntry {
            circuit_id: Some("circuit-1".to_string()),
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
            recent_rtt_data: [crate::throughput_tracker::flow_data::RttData::from_nanos(0); 60],
            last_fresh_rtt_data_cycle: 0,
            last_seen: 0,
            tcp_retransmits: DownUpOrder::zeroed(),
            tcp_retransmit_packets: DownUpOrder::zeroed(),
            qoq: QoqScores::default(),
        };

        let circuit_hash = resolved_circuit_hash_for_submission(&catalog, &ip, &entry);

        assert_eq!(circuit_hash, Some(hash_to_i64("circuit-1")));
    }

 /// 3b DEFECT 2 regression: loaded ingress DEMAND above the cap
    /// (1.11 Gbps actual vs 200 Mbps shaped on a 1 Gbps link) must still
    /// submit; only shaped corruption above the cap trips the guard.
    #[test]
    fn bridge_spike_guard_ignores_ingress_demand_and_catches_shaped_corruption() {
        // The rig case: ~1.11 Gbps ingress demand, ~200 Mbps shaped, 1 Gbps cap.
        assert!(
            !bridge_spike_guard(24_993_125, 0, 1_000, 1_000),
            "loaded ingress demand must not suppress the submission"
        );
        // Shaped at exactly the cap stays clean.
        assert!(
            !bridge_spike_guard(125_000_000, 0, 1_000, 1_000),
            "shaped at the cap is not a spike"
        );
        // Shaped above the cap is impossible by construction — corruption.
        assert!(
            bridge_spike_guard(200_000_000, 0, 1_000, 1_000),
            "shaped above the cap trips the guard"
        );
        assert!(bridge_spike_guard(0, 200_000_000, 1_000, 1_000));
    }

 /// 3b: the topology push fires only on a hash change (startup
    /// included) and is silent on unchanged collation ticks.
    #[test]
    fn topology_push_gate_fires_only_on_hash_change() {
        assert!(
            !topology_hash_changed(7, 7),
            "unchanged hash must not resend"
        );
        assert!(topology_hash_changed(7, 8), "changed hash fires");
        assert!(
            topology_hash_changed(0, 8),
            "first push (stored sentinel -> real hash) fires"
        );
    }

 /// 3b: the ingest flush fires only after a completed submission —
    /// a guard/gate bail (returns false) never flushes.
    #[test]
    fn lts_flush_only_after_a_completed_submission() {
        assert!(lts_flush_after(true), "a completed batch flushes");
        assert!(
            !lts_flush_after(false),
            "a skipped/suppressed submission must not flush"
        );
    }
}
