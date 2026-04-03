use crate::shaped_devices_tracker::{SHAPED_DEVICE_HASH_CACHE, SHAPED_DEVICES};
use crate::throughput_tracker::flow_data::{ALL_FLOWS, FlowbeeLocalData, get_asn_name_and_country};
use lqos_utils::hash_to_i64;
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample};
use lqos_utils::unix_time::time_since_boot;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::time::Duration;

const RECENT_CIRCUIT_FLOWS_WINDOW_NANOS: u64 = 30 * 1_000_000_000;
const SANKEY_RECENT_FLOW_WINDOW_NANOS: u64 = 10 * 1_000_000_000;
const SANKEY_TOP_FLOW_LIMIT: usize = 20;
const TOP_ASN_LIMIT: usize = 10;
const FLOW_RATE_SANITY_MULTIPLIER: f64 = 2.0;
const FLOW_RATE_SANITY_FLOOR_BPS: u64 = 25_000_000;
const TRAFFIC_FLOW_HIDE_THRESHOLD_BPS: u32 = 1_048_576;

/// Lightweight live summary for the circuit page header and Queue Dynamics.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitSummaryData {
    pub circuit_id: String,
    pub bytes_per_second: DownUpOrder<u64>,
    pub rtt_current_p50_nanos: DownUpOrder<Option<u64>>,
    pub tcp_retransmit_sample: DownUpOrder<TcpRetransmitSample>,
    pub qoo_score: Option<f32>,
    pub rtt_excluded: bool,
    pub active_flow_count: usize,
    pub active_asn_count: usize,
}

/// Server-side query for the live `Traffic Flows` table.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitTrafficFlowsQuery {
    pub circuit: String,
    pub page: usize,
    pub page_size: usize,
    pub hide_small: bool,
    pub sort_column: String,
    pub sort_direction: String,
}

/// Compact server-side row for the live `Traffic Flows` table.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitTrafficFlowRow {
    pub protocol_name: String,
    pub down_bps: u32,
    pub up_bps: u32,
    pub bytes_sent_down: u64,
    pub bytes_sent_up: u64,
    pub packets_sent_down: u64,
    pub packets_sent_up: u64,
    pub tcp_retransmits_down: u16,
    pub tcp_retransmits_up: u16,
    pub retransmit_down_pct: f64,
    pub retransmit_up_pct: f64,
    pub rtt_down_nanos: u64,
    pub rtt_up_nanos: u64,
    pub qoo_down: Option<f32>,
    pub qoo_up: Option<f32>,
    pub asn_name: String,
    pub asn_country: String,
    pub remote_ip: String,
    pub opacity: f64,
    pub sort_rate_bps: f64,
}

/// Page of server-side circuit traffic-flow rows.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitTrafficFlowsPage {
    pub query: CircuitTrafficFlowsQuery,
    pub total_rows: usize,
    pub rows: Vec<CircuitTrafficFlowRow>,
}

/// Server-side query for the circuit `Top ASNs` table.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitTopAsnsQuery {
    pub circuit: String,
    pub hide_small: bool,
}

/// Aggregated row for the circuit `Top ASNs` table, including recent rate,
/// median RTT/QoO, retransmit, and flow-count context for a circuit-local ASN.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitTopAsnRow {
    pub asn_name: String,
    pub asn_country: String,
    pub down_bps: u64,
    pub up_bps: u64,
    pub rtt_down_nanos: u64,
    pub rtt_up_nanos: u64,
    pub qoo_down: Option<f32>,
    pub qoo_up: Option<f32>,
    pub retransmit_down_pct: f64,
    pub retransmit_up_pct: f64,
    pub flow_count: usize,
}

/// Server-side payload for the circuit `Top ASNs` table.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitTopAsnsData {
    pub total_asns: usize,
    pub rows: Vec<CircuitTopAsnRow>,
}

/// Compact flow row for the circuit `Flow Sankey` tab.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitFlowSankeyRow {
    pub device_name: String,
    pub asn_id: u32,
    pub asn_name: String,
    pub protocol_name: String,
    pub remote_ip: String,
    pub down_bps: u32,
    pub up_bps: u32,
    pub last_seen_nanos: u64,
}

#[derive(Clone, Debug)]
struct CircuitFlowSnapshotRow {
    device_name: String,
    asn_id: u32,
    asn_name: String,
    asn_country: String,
    protocol_name: String,
    remote_ip: String,
    down_bps: u32,
    up_bps: u32,
    bytes_sent_down: u64,
    bytes_sent_up: u64,
    packets_sent_down: u64,
    packets_sent_up: u64,
    tcp_retransmits_down: u16,
    tcp_retransmits_up: u16,
    retransmit_down_pct: f64,
    retransmit_up_pct: f64,
    rtt_down_nanos: u64,
    rtt_up_nanos: u64,
    qoo_down: Option<f32>,
    qoo_up: Option<f32>,
    last_seen_nanos: u64,
    opacity: f64,
    sort_rate_bps: f64,
}

const fn clamp_u64_to_u32(value: u64) -> u32 {
    if value > u32::MAX as u64 {
        u32::MAX
    } else {
        value as u32
    }
}

fn sanitized_plan_ceiling_bps(plan_mbps: f32) -> u32 {
    if !plan_mbps.is_finite() || plan_mbps <= 0.0 {
        return clamp_u64_to_u32(FLOW_RATE_SANITY_FLOOR_BPS);
    }

    let scaled = (plan_mbps as f64 * 1_000_000.0 * FLOW_RATE_SANITY_MULTIPLIER).round();
    let scaled = scaled.max(FLOW_RATE_SANITY_FLOOR_BPS as f64) as u64;
    clamp_u64_to_u32(scaled)
}

fn circuit_display_rate_ceiling_bps(
    shaped: &lqos_config::ConfigShapedDevices,
    circuit_hash: i64,
) -> Option<DownUpOrder<u32>> {
    let mut max_down_mbps = 0.0_f32;
    let mut max_up_mbps = 0.0_f32;

    for device in &shaped.devices {
        if device.circuit_hash != circuit_hash {
            continue;
        }
        max_down_mbps = max_down_mbps.max(device.download_max_mbps);
        max_up_mbps = max_up_mbps.max(device.upload_max_mbps);
    }

    if max_down_mbps <= 0.0 && max_up_mbps <= 0.0 {
        return None;
    }

    Some(DownUpOrder {
        down: sanitized_plan_ceiling_bps(max_down_mbps),
        up: sanitized_plan_ceiling_bps(max_up_mbps),
    })
}

fn display_rate_bps(
    local: &FlowbeeLocalData,
    ceiling_bps: Option<DownUpOrder<u32>>,
) -> DownUpOrder<u32> {
    let Some(ceiling_bps) = ceiling_bps else {
        return local.rate_estimate_bps;
    };

    DownUpOrder {
        down: local.rate_estimate_bps.down.min(ceiling_bps.down),
        up: local.rate_estimate_bps.up.min(ceiling_bps.up),
    }
}

fn flow_rtt_nanos(local: &FlowbeeLocalData) -> DownUpOrder<u64> {
    let rtt = local.get_rtt_array();
    DownUpOrder {
        down: rtt[0].as_nanos(),
        up: rtt[1].as_nanos(),
    }
}

fn flow_qoo(local: &FlowbeeLocalData) -> DownUpOrder<Option<f32>> {
    let qoq = local.get_qoq_scores();
    DownUpOrder {
        down: qoq.download_total_f32(),
        up: qoq.upload_total_f32(),
    }
}

fn flow_snapshot_rows(circuit_id: &str) -> Vec<CircuitFlowSnapshotRow> {
    let circuit_hash = hash_to_i64(circuit_id);
    let shaped = SHAPED_DEVICES.load();
    let cache = SHAPED_DEVICE_HASH_CACHE.load();
    let display_rate_ceiling = circuit_display_rate_ceiling_bps(&shaped, circuit_hash);
    let Ok(now) = time_since_boot() else {
        return Vec::new();
    };
    let now_as_nanos = Duration::from(now).as_nanos() as u64;
    let recent_cutoff = now_as_nanos.saturating_sub(RECENT_CIRCUIT_FLOWS_WINDOW_NANOS);

    let all_flows = ALL_FLOWS.lock();
    all_flows
        .flow_data
        .iter()
        .filter_map(|(key, (local, analysis))| {
            if local.last_seen < recent_cutoff {
                return None;
            }
            if local.circuit_hash != Some(circuit_hash) {
                return None;
            }

            let device_name = local
                .device_hash
                .and_then(|hash| cache.index_by_device_hash(&shaped, hash))
                .and_then(|idx| shaped.devices.get(idx))
                .map(|d| d.device_name.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            let geo = get_asn_name_and_country(key.remote_ip.as_ip());
            let display_rate = display_rate_bps(local, display_rate_ceiling);
            let current_rate = display_rate.down as u64 + display_rate.up as u64;
            let packets_sent_down = local.packets_sent.down;
            let packets_sent_up = local.packets_sent.up;
            let tcp_retransmits_down = local.tcp_retransmits.down;
            let tcp_retransmits_up = local.tcp_retransmits.up;
            let retransmit_down_pct = if tcp_retransmits_down > 0 && packets_sent_down > 0 {
                tcp_retransmits_down as f64 / packets_sent_down as f64
            } else {
                0.0
            };
            let retransmit_up_pct = if tcp_retransmits_up > 0 && packets_sent_up > 0 {
                tcp_retransmits_up as f64 / packets_sent_up as f64
            } else {
                0.0
            };
            let rtt = flow_rtt_nanos(local);
            let qoo = flow_qoo(local);
            let last_seen_nanos = now_as_nanos.saturating_sub(local.last_seen);

            Some(CircuitFlowSnapshotRow {
                device_name,
                asn_id: analysis.asn_id.0,
                asn_name: geo.name,
                asn_country: geo.country,
                protocol_name: analysis.protocol_analysis.to_string(),
                remote_ip: key.remote_ip.to_string(),
                down_bps: display_rate.down,
                up_bps: display_rate.up,
                bytes_sent_down: local.bytes_sent.down,
                bytes_sent_up: local.bytes_sent.up,
                packets_sent_down,
                packets_sent_up,
                tcp_retransmits_down,
                tcp_retransmits_up,
                retransmit_down_pct,
                retransmit_up_pct,
                rtt_down_nanos: rtt.down,
                rtt_up_nanos: rtt.up,
                qoo_down: qoo.down,
                qoo_up: qoo.up,
                last_seen_nanos,
                opacity: 1.0
                    - f64::min(
                        1.0,
                        last_seen_nanos as f64 / RECENT_CIRCUIT_FLOWS_WINDOW_NANOS as f64,
                    ),
                sort_rate_bps: current_rate as f64,
            })
        })
        .collect()
}

fn sort_direction_is_asc(sort_direction: &str) -> bool {
    sort_direction.eq_ignore_ascii_case("asc")
}

fn compare_f64(left: f64, right: f64, asc: bool) -> Ordering {
    let order = left.partial_cmp(&right).unwrap_or(Ordering::Equal);
    if asc { order } else { order.reverse() }
}

fn compare_u64(left: u64, right: u64, asc: bool) -> Ordering {
    if asc {
        left.cmp(&right)
    } else {
        right.cmp(&left)
    }
}

fn compare_strings(left: &str, right: &str, asc: bool) -> Ordering {
    if asc {
        left.cmp(right)
    } else {
        right.cmp(left)
    }
}

fn median_u64(values: &mut [u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let midpoint = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[midpoint])
    } else {
        let left = values[midpoint - 1] as u128;
        let right = values[midpoint] as u128;
        Some(((left + right) / 2) as u64)
    }
}

fn median_f32(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|left, right| left.total_cmp(right));
    let midpoint = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[midpoint])
    } else {
        Some((values[midpoint - 1] + values[midpoint]) / 2.0)
    }
}

fn sort_traffic_rows(rows: &mut [CircuitFlowSnapshotRow], sort_column: &str, sort_direction: &str) {
    let asc = sort_direction_is_asc(sort_direction);
    rows.sort_by(|a, b| {
        let primary = match sort_column {
            "protocol" => compare_strings(&a.protocol_name, &b.protocol_name, asc),
            "bytes" => compare_u64(
                a.bytes_sent_down + a.bytes_sent_up,
                b.bytes_sent_down + b.bytes_sent_up,
                asc,
            ),
            "packets" => compare_u64(
                a.packets_sent_down + a.packets_sent_up,
                b.packets_sent_down + b.packets_sent_up,
                asc,
            ),
            "retransmits" => compare_f64(
                a.retransmit_down_pct + a.retransmit_up_pct,
                b.retransmit_down_pct + b.retransmit_up_pct,
                asc,
            ),
            "rtt" => compare_u64(
                a.rtt_down_nanos + a.rtt_up_nanos,
                b.rtt_down_nanos + b.rtt_up_nanos,
                asc,
            ),
            "qoo" => compare_f64(
                a.qoo_down.unwrap_or(0.0) as f64 + a.qoo_up.unwrap_or(0.0) as f64,
                b.qoo_down.unwrap_or(0.0) as f64 + b.qoo_up.unwrap_or(0.0) as f64,
                asc,
            ),
            "asn" => compare_strings(&a.asn_name, &b.asn_name, asc),
            "country" => compare_strings(&a.asn_country, &b.asn_country, asc),
            "ip" => compare_strings(&a.remote_ip, &b.remote_ip, asc),
            _ => compare_f64(a.sort_rate_bps, b.sort_rate_bps, asc),
        };
        if primary == Ordering::Equal {
            compare_f64(a.sort_rate_bps, b.sort_rate_bps, false)
        } else {
            primary
        }
    });
}

pub fn circuit_flow_counts(circuit_id: &str) -> (usize, usize) {
    let rows = flow_snapshot_rows(circuit_id);
    let asn_count = rows
        .iter()
        .map(|row| (row.asn_name.clone(), row.asn_country.clone()))
        .collect::<std::collections::HashSet<_>>()
        .len();
    (rows.len(), asn_count)
}

pub fn circuit_traffic_flows_page(query: &CircuitTrafficFlowsQuery) -> CircuitTrafficFlowsPage {
    let mut rows = flow_snapshot_rows(&query.circuit);
    if query.hide_small {
        rows.retain(|row| {
            row.down_bps > TRAFFIC_FLOW_HIDE_THRESHOLD_BPS
                || row.up_bps > TRAFFIC_FLOW_HIDE_THRESHOLD_BPS
        });
    }
    sort_traffic_rows(&mut rows, &query.sort_column, &query.sort_direction);

    let total_rows = rows.len();
    let page_size = query.page_size.max(1);
    let page = query.page.max(1);
    let start = page_size.saturating_mul(page.saturating_sub(1));
    let paged = rows
        .into_iter()
        .skip(start)
        .take(page_size)
        .map(|row| CircuitTrafficFlowRow {
            protocol_name: row.protocol_name,
            down_bps: row.down_bps,
            up_bps: row.up_bps,
            bytes_sent_down: row.bytes_sent_down,
            bytes_sent_up: row.bytes_sent_up,
            packets_sent_down: row.packets_sent_down,
            packets_sent_up: row.packets_sent_up,
            tcp_retransmits_down: row.tcp_retransmits_down,
            tcp_retransmits_up: row.tcp_retransmits_up,
            retransmit_down_pct: row.retransmit_down_pct,
            retransmit_up_pct: row.retransmit_up_pct,
            rtt_down_nanos: row.rtt_down_nanos,
            rtt_up_nanos: row.rtt_up_nanos,
            qoo_down: row.qoo_down,
            qoo_up: row.qoo_up,
            asn_name: row.asn_name,
            asn_country: row.asn_country,
            remote_ip: row.remote_ip,
            opacity: row.opacity,
            sort_rate_bps: row.sort_rate_bps,
        })
        .collect();

    CircuitTrafficFlowsPage {
        query: query.clone(),
        total_rows,
        rows: paged,
    }
}

pub fn circuit_top_asns_data(query: &CircuitTopAsnsQuery) -> CircuitTopAsnsData {
    #[derive(Default)]
    struct AsnBucket {
        asn_name: String,
        asn_country: String,
        down_bps: u64,
        up_bps: u64,
        rtt_down_nanos: Vec<u64>,
        rtt_up_nanos: Vec<u64>,
        qoo_down: Vec<f32>,
        qoo_up: Vec<f32>,
        packets_sent_down: u64,
        packets_sent_up: u64,
        tcp_retransmits_down: u64,
        tcp_retransmits_up: u64,
        flow_count: usize,
    }

    let mut rows = flow_snapshot_rows(&query.circuit);
    if query.hide_small {
        rows.retain(|row| {
            row.down_bps > TRAFFIC_FLOW_HIDE_THRESHOLD_BPS
                || row.up_bps > TRAFFIC_FLOW_HIDE_THRESHOLD_BPS
        });
    }

    let mut buckets: fxhash::FxHashMap<u32, AsnBucket> = fxhash::FxHashMap::default();
    for row in rows {
        let bucket = buckets.entry(row.asn_id).or_insert_with(|| AsnBucket {
            asn_name: if row.asn_name.trim().is_empty() {
                "Unknown ASN".to_string()
            } else {
                row.asn_name.clone()
            },
            asn_country: row.asn_country.clone(),
            down_bps: 0,
            up_bps: 0,
            rtt_down_nanos: Vec::new(),
            rtt_up_nanos: Vec::new(),
            qoo_down: Vec::new(),
            qoo_up: Vec::new(),
            packets_sent_down: 0,
            packets_sent_up: 0,
            tcp_retransmits_down: 0,
            tcp_retransmits_up: 0,
            flow_count: 0,
        });
        if bucket.asn_name == "Unknown ASN" && !row.asn_name.trim().is_empty() {
            bucket.asn_name = row.asn_name.clone();
        }
        if bucket.asn_country.trim().is_empty() && !row.asn_country.trim().is_empty() {
            bucket.asn_country = row.asn_country.clone();
        }
        bucket.down_bps += row.down_bps as u64;
        bucket.up_bps += row.up_bps as u64;
        bucket.packets_sent_down += row.packets_sent_down;
        bucket.packets_sent_up += row.packets_sent_up;
        bucket.tcp_retransmits_down += row.tcp_retransmits_down as u64;
        bucket.tcp_retransmits_up += row.tcp_retransmits_up as u64;
        if row.rtt_down_nanos > 0 {
            bucket.rtt_down_nanos.push(row.rtt_down_nanos);
        }
        if row.rtt_up_nanos > 0 {
            bucket.rtt_up_nanos.push(row.rtt_up_nanos);
        }
        if let Some(qoo_down) = row.qoo_down {
            bucket.qoo_down.push(qoo_down);
        }
        if let Some(qoo_up) = row.qoo_up {
            bucket.qoo_up.push(qoo_up);
        }
        bucket.flow_count += 1;
    }

    let total_asns = buckets.len();
    let mut bucket_rows: Vec<CircuitTopAsnRow> = buckets
        .into_values()
        .map(|mut row| CircuitTopAsnRow {
            asn_name: row.asn_name,
            asn_country: row.asn_country,
            down_bps: row.down_bps,
            up_bps: row.up_bps,
            rtt_down_nanos: median_u64(&mut row.rtt_down_nanos).unwrap_or_default(),
            rtt_up_nanos: median_u64(&mut row.rtt_up_nanos).unwrap_or_default(),
            qoo_down: median_f32(&mut row.qoo_down),
            qoo_up: median_f32(&mut row.qoo_up),
            retransmit_down_pct: if row.packets_sent_down > 0 {
                row.tcp_retransmits_down as f64 / row.packets_sent_down as f64
            } else {
                0.0
            },
            retransmit_up_pct: if row.packets_sent_up > 0 {
                row.tcp_retransmits_up as f64 / row.packets_sent_up as f64
            } else {
                0.0
            },
            flow_count: row.flow_count,
        })
        .collect();

    bucket_rows.sort_by(|a, b| {
        let a_rate = a.down_bps + a.up_bps;
        let b_rate = b.down_bps + b.up_bps;
        b_rate
            .cmp(&a_rate)
            .then_with(|| b.flow_count.cmp(&a.flow_count))
            .then_with(|| a.asn_name.cmp(&b.asn_name))
    });
    bucket_rows.truncate(TOP_ASN_LIMIT);

    CircuitTopAsnsData {
        total_asns,
        rows: bucket_rows,
    }
}

pub fn circuit_flow_sankey_rows(circuit_id: &str) -> Vec<CircuitFlowSankeyRow> {
    let mut rows = flow_snapshot_rows(circuit_id);
    rows.retain(|row| row.last_seen_nanos <= SANKEY_RECENT_FLOW_WINDOW_NANOS);
    rows.sort_by(|a, b| compare_f64(a.sort_rate_bps, b.sort_rate_bps, false));
    rows.into_iter()
        .take(SANKEY_TOP_FLOW_LIMIT)
        .map(|row| CircuitFlowSankeyRow {
            device_name: row.device_name,
            asn_id: row.asn_id,
            asn_name: row.asn_name,
            protocol_name: row.protocol_name,
            remote_ip: row.remote_ip,
            down_bps: row.down_bps,
            up_bps: row.up_bps,
            last_seen_nanos: row.last_seen_nanos,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{median_f32, median_u64};

    #[test]
    fn median_u64_handles_even_and_odd_lengths() {
        let mut odd = vec![30_u64, 10, 20];
        assert_eq!(median_u64(&mut odd), Some(20));

        let mut even = vec![40_u64, 10, 30, 20];
        assert_eq!(median_u64(&mut even), Some(25));
    }

    #[test]
    fn median_f32_handles_even_and_odd_lengths() {
        let mut odd = vec![30.0_f32, 10.0, 20.0];
        assert_eq!(median_f32(&mut odd), Some(20.0));

        let mut even = vec![40.0_f32, 10.0, 30.0, 20.0];
        assert_eq!(median_f32(&mut even), Some(25.0));
    }
}
