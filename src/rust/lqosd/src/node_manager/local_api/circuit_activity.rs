use crate::throughput_tracker::flow_data::{
    ActiveFlowDisplayFields, ActiveFlowSnapshot, for_each_active_flow_for_circuit,
};
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
    #[serde(default)]
    pub actual_bytes_per_second: DownUpOrder<u64>,
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
    /// Legacy UI field name; payload value is the flow age at snapshot time.
    #[serde(rename = "last_seen_nanos")]
    pub age_nanos_wire: u64,
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
    age_nanos: u64,
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
    catalog: &lqos_network_devices::NetworkDevicesCatalog,
    circuit_hash: i64,
) -> Option<DownUpOrder<u32>> {
    let mut max_down_mbps = 0.0_f32;
    let mut max_up_mbps = 0.0_f32;

    for device in catalog.iter_all_devices() {
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
    flow: &ActiveFlowSnapshot,
    ceiling_bps: Option<DownUpOrder<u32>>,
) -> DownUpOrder<u32> {
    let Some(ceiling_bps) = ceiling_bps else {
        return flow.rate_estimate_bps;
    };

    DownUpOrder {
        down: flow.rate_estimate_bps.down.min(ceiling_bps.down),
        up: flow.rate_estimate_bps.up.min(ceiling_bps.up),
    }
}

fn circuit_flow_snapshot_row_from_flow(
    flow: &ActiveFlowSnapshot,
    device_name: String,
    display: &ActiveFlowDisplayFields,
    display_rate_ceiling: Option<DownUpOrder<u32>>,
    now_as_nanos: u64,
) -> CircuitFlowSnapshotRow {
    let display_rate = display_rate_bps(flow, display_rate_ceiling);
    let current_rate = display_rate.down as u64 + display_rate.up as u64;
    let packets_sent_down = flow.packets_sent.down;
    let packets_sent_up = flow.packets_sent.up;
    let tcp_retransmits_down = flow.tcp_retransmits.down;
    let tcp_retransmits_up = flow.tcp_retransmits.up;
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
    let age_nanos = flow.age_nanos(now_as_nanos);

    CircuitFlowSnapshotRow {
        device_name,
        asn_id: display.remote_asn,
        asn_name: display.remote_asn_name.clone(),
        asn_country: display.remote_asn_country.clone(),
        protocol_name: display.analysis.clone(),
        remote_ip: display.remote_ip.clone(),
        down_bps: display_rate.down,
        up_bps: display_rate.up,
        bytes_sent_down: flow.bytes_sent.down,
        bytes_sent_up: flow.bytes_sent.up,
        packets_sent_down,
        packets_sent_up,
        tcp_retransmits_down,
        tcp_retransmits_up,
        retransmit_down_pct,
        retransmit_up_pct,
        rtt_down_nanos: flow.rtt_nanos.down,
        rtt_up_nanos: flow.rtt_nanos.up,
        qoo_down: flow.qoo.down,
        qoo_up: flow.qoo.up,
        age_nanos,
        opacity: 1.0
            - f64::min(
                1.0,
                age_nanos as f64 / RECENT_CIRCUIT_FLOWS_WINDOW_NANOS as f64,
            ),
        sort_rate_bps: current_rate as f64,
    }
}

fn current_and_recent_cutoff_nanos() -> Option<(u64, u64)> {
    let now = time_since_boot().ok()?;
    let now_as_nanos = Duration::from(now).as_nanos() as u64;
    let recent_cutoff = now_as_nanos.saturating_sub(RECENT_CIRCUIT_FLOWS_WINDOW_NANOS);
    Some((now_as_nanos, recent_cutoff))
}

fn flow_snapshot_rows(circuit_id: &str) -> Vec<CircuitFlowSnapshotRow> {
    let circuit_hash = hash_to_i64(circuit_id);
    let catalog = lqos_network_devices::network_devices_catalog();
    let display_rate_ceiling = circuit_display_rate_ceiling_bps(&catalog, circuit_hash);
    let Some((now_as_nanos, recent_cutoff)) = current_and_recent_cutoff_nanos() else {
        return Vec::new();
    };

    let mut rows = Vec::new();
    for_each_active_flow_for_circuit(circuit_hash, recent_cutoff, |flow| {
        let device_name = if flow.device_name.is_empty() {
            "Unknown".to_string()
        } else {
            flow.device_name.clone()
        };

        let row = circuit_flow_snapshot_row_from_flow(
            flow,
            device_name,
            &flow.display,
            display_rate_ceiling,
            now_as_nanos,
        );
        rows.push(row);
    });
    rows
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
    let circuit_hash = hash_to_i64(circuit_id);
    let Some((_, recent_cutoff)) = current_and_recent_cutoff_nanos() else {
        return (0, 0);
    };

    let mut flow_count = 0;
    let mut asns = std::collections::HashSet::new();
    for_each_active_flow_for_circuit(circuit_hash, recent_cutoff, |flow| {
        flow_count += 1;
        asns.insert(flow.display.remote_asn);
    });

    (flow_count, asns.len())
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
    rows.retain(|row| row.age_nanos <= SANKEY_RECENT_FLOW_WINDOW_NANOS);
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
            age_nanos_wire: row.age_nanos,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        CircuitFlowSankeyRow, CircuitTopAsnsQuery, CircuitTrafficFlowsQuery,
        circuit_flow_counts, circuit_flow_sankey_rows, circuit_flow_snapshot_row_from_flow,
        circuit_top_asns_data, circuit_traffic_flows_page, flow_snapshot_rows, median_f32,
        median_u64,
    };
    use crate::test_support::{ActiveFlowSnapshotTestContext, active_flow_entry};
    use crate::throughput_tracker::flow_data::{
        ActiveFlowDisplayFields, ActiveFlowSnapshot, AsnId, replace_active_flows_for_test,
        replace_active_flows_live_for_test,
    };
    use lqos_config::ShapedDevice;
    use lqos_sys::flowbee_data::FlowbeeKey;
    use lqos_utils::units::DownUpOrder;
    use lqos_utils::{XdpIpAddress, hash_to_i64};
    use std::net::{IpAddr, Ipv4Addr};

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

    #[test]
    fn circuit_flow_sankey_row_decodes_legacy_last_seen_nanos_wire_field() {
        let row: CircuitFlowSankeyRow = serde_json::from_value(serde_json::json!({
            "device_name": "Device Public",
            "asn_id": 64512,
            "asn_name": "Example ASN",
            "protocol_name": "HTTPS",
            "remote_ip": "198.51.100.10",
            "down_bps": 70_000_000,
            "up_bps": 2_000_000,
            "last_seen_nanos": 2_000_000_000_u64
        }))
        .expect("legacy circuit flow row should deserialize");

        assert_eq!(row.age_nanos_wire, 2_000_000_000);
    }

    #[test]
    fn public_circuit_flow_readers_use_published_snapshot_and_catalog_matching() {
        let _ctx = ActiveFlowSnapshotTestContext::with_shaped_devices(
            "circuit-activity-test",
            vec![ShapedDevice {
                circuit_id: "circuit-public".to_string(),
                circuit_name: "Circuit Public".to_string(),
                device_id: "device-public".to_string(),
                device_name: "Device Public".to_string(),
                parent_node: "Parent".to_string(),
                ipv4: vec![(Ipv4Addr::new(192, 0, 2, 42), 32)],
                download_max_mbps: 100.0,
                upload_max_mbps: 50.0,
                ..Default::default()
            }],
        );
        let now_nanos =
            std::time::Duration::from(lqos_utils::unix_time::time_since_boot().unwrap())
                .as_nanos() as u64;
        let fresh = now_nanos.saturating_sub(2 * 1_000_000_000);
        let fresh_small = now_nanos.saturating_sub(4 * 1_000_000_000);
        let stale = now_nanos.saturating_sub(40 * 1_000_000_000);
        let circuit_hash = hash_to_i64("circuit-public");

        let ip_matched = active_flow_entry(
            [192, 0, 2, 42],
            [198, 51, 100, 10],
            50_000,
            fresh,
            DownUpOrder::new(70_000_000, 2_000_000),
            DownUpOrder::new(5_000, 1_000),
            DownUpOrder::new(50, 10),
        );
        let small = active_flow_entry(
            [192, 0, 2, 42],
            [198, 51, 100, 11],
            50_001,
            fresh_small,
            DownUpOrder::new(1_000, 1_000),
            DownUpOrder::new(12_000, 1_000),
            DownUpOrder::new(120, 10),
        );
        let mut hash_matched = active_flow_entry(
            [192, 0, 2, 99],
            [198, 51, 100, 14],
            50_004,
            fresh,
            DownUpOrder::new(30_000_000, 1_000_000),
            DownUpOrder::new(8_000, 1_000),
            DownUpOrder::new(80, 10),
        );
        hash_matched.1.0.circuit_hash = Some(circuit_hash);
        let stale_flow = active_flow_entry(
            [192, 0, 2, 42],
            [198, 51, 100, 12],
            50_002,
            stale,
            DownUpOrder::new(90_000_000, 1_000_000),
            DownUpOrder::new(90_000, 1_000),
            DownUpOrder::new(900, 10),
        );
        let other_circuit = active_flow_entry(
            [192, 0, 2, 43],
            [198, 51, 100, 13],
            50_003,
            fresh,
            DownUpOrder::new(80_000_000, 1_000_000),
            DownUpOrder::new(80_000, 1_000),
            DownUpOrder::new(800, 10),
        );
        replace_active_flows_for_test(vec![
            ip_matched,
            small,
            hash_matched,
            stale_flow,
            other_circuit,
        ]);

        let page = circuit_traffic_flows_page(&CircuitTrafficFlowsQuery {
            circuit: "circuit-public".to_string(),
            page: 1,
            page_size: 10,
            hide_small: false,
            sort_column: "bytes".to_string(),
            sort_direction: "desc".to_string(),
        });
        assert_eq!(page.total_rows, 3);
        assert_eq!(
            page.rows
                .iter()
                .map(|row| row.remote_ip.as_str())
                .collect::<Vec<_>>(),
            vec!["198.51.100.11", "198.51.100.14", "198.51.100.10"]
        );

        let hidden_small = circuit_traffic_flows_page(&CircuitTrafficFlowsQuery {
            circuit: "circuit-public".to_string(),
            page: 1,
            page_size: 10,
            hide_small: true,
            sort_column: "rate".to_string(),
            sort_direction: "desc".to_string(),
        });
        assert_eq!(hidden_small.total_rows, 2);
        assert_eq!(hidden_small.rows[0].remote_ip, "198.51.100.10");
        assert_eq!(hidden_small.rows[1].remote_ip, "198.51.100.14");

        let top_asns = circuit_top_asns_data(&CircuitTopAsnsQuery {
            circuit: "circuit-public".to_string(),
            hide_small: true,
        });
        assert_eq!(top_asns.total_asns, 1);
        assert_eq!(top_asns.rows[0].flow_count, 2);
        assert_eq!(top_asns.rows[0].down_bps, 100_000_000);

        let sankey_rows = circuit_flow_sankey_rows("circuit-public");
        assert_eq!(sankey_rows[0].device_name, "Device Public");
        let sankey_wire = serde_json::to_value(&sankey_rows[0]).expect("row should serialize");
        assert!(sankey_wire.get("last_seen_nanos").is_some());
        assert!(sankey_wire.get("age_nanos").is_none());
        assert_eq!(
            sankey_rows
                .iter()
                .map(|row| row.remote_ip.as_str())
                .collect::<Vec<_>>(),
            vec!["198.51.100.10", "198.51.100.14", "198.51.100.11"]
        );

        replace_active_flows_live_for_test(Vec::new());

        let stale_page = circuit_traffic_flows_page(&CircuitTrafficFlowsQuery {
            circuit: "circuit-public".to_string(),
            page: 1,
            page_size: 10,
            hide_small: false,
            sort_column: "bytes".to_string(),
            sort_direction: "desc".to_string(),
        });
        assert_eq!(stale_page.total_rows, 3);
        assert_eq!(circuit_flow_sankey_rows("circuit-public").len(), 3);
    }

    #[test]
    fn circuit_flow_counts_uses_numeric_asn_identity() {
        let _ctx = ActiveFlowSnapshotTestContext::with_shaped_devices(
            "circuit-asn-count-test",
            vec![ShapedDevice {
                circuit_id: "circuit-asn-count".to_string(),
                circuit_name: "Circuit ASN Count".to_string(),
                device_id: "device-asn-count".to_string(),
                device_name: "Device ASN Count".to_string(),
                parent_node: "Parent".to_string(),
                ipv4: vec![(Ipv4Addr::new(192, 0, 2, 0), 24)],
                ..Default::default()
            }],
        );
        let now_nanos =
            std::time::Duration::from(lqos_utils::unix_time::time_since_boot().unwrap())
                .as_nanos() as u64;
        let fresh = now_nanos.saturating_sub(1_000_000_000);

        let mut first = active_flow_entry(
            [192, 0, 2, 42],
            [198, 51, 100, 10],
            50_000,
            fresh,
            DownUpOrder::new(70_000_000, 2_000_000),
            DownUpOrder::new(5_000, 1_000),
            DownUpOrder::new(50, 10),
        );
        first.1.1.asn_id = AsnId(64_512);
        let mut second = active_flow_entry(
            [192, 0, 2, 43],
            [198, 51, 100, 11],
            50_001,
            fresh,
            DownUpOrder::new(30_000_000, 1_000_000),
            DownUpOrder::new(8_000, 1_000),
            DownUpOrder::new(80, 10),
        );
        second.1.1.asn_id = AsnId(64_513);
        replace_active_flows_for_test(vec![first, second]);

        let rows = flow_snapshot_rows("circuit-asn-count");
        let display_identities = rows
            .iter()
            .map(|row| (row.asn_name.as_str(), row.asn_country.as_str()))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(display_identities.len(), 1);
        assert_eq!(circuit_flow_counts("circuit-asn-count"), (2, 2));
    }

    #[test]
    fn circuit_flow_snapshot_row_uses_cached_flow_fields_and_clamps_display_rate() {
        let mut key = FlowbeeKey::default();
        key.remote_ip = XdpIpAddress::from_ip(IpAddr::from([198, 51, 100, 20]));
        key.ip_protocol = 6;
        key.src_port = 443;
        key.dst_port = 50_000;
        let flow = ActiveFlowSnapshot {
            key,
            display: ActiveFlowDisplayFields {
                remote_ip: "198.51.100.20".to_string(),
                local_ip: "192.0.2.10".to_string(),
                src_port: 443,
                dst_port: 50_000,
                ip_protocol: lqos_bus::FlowbeeProtocol::TCP,
                remote_asn: 0,
                remote_asn_name: "Example ASN".to_string(),
                remote_asn_country: "US".to_string(),
                analysis: "HTTPS".to_string(),
            },
            bytes_sent: DownUpOrder::new(10_000, 20_000),
            packets_sent: DownUpOrder::new(100, 200),
            rate_estimate_bps: DownUpOrder::new(100_000_000, 15_000_000),
            tcp_retransmits: DownUpOrder::new(5, 4),
            end_status: 0,
            tos: 0,
            flags: 0x12,
            circuit_hash: None,
            device_hash: None,
            circuit_id: String::new(),
            circuit_name: String::new(),
            device_name: "Device A".to_string(),
            last_seen: 70_000_000_000,
            start_time: 65_000_000_000,
            rtt_nanos: DownUpOrder::new(12_000_000, 34_000_000),
            qoo: DownUpOrder::new(Some(77.0), Some(66.0)),
        };

        let row = circuit_flow_snapshot_row_from_flow(
            &flow,
            "Device A".to_string(),
            &flow.display,
            Some(DownUpOrder::new(50_000_000, 25_000_000)),
            75_000_000_000,
        );

        assert_eq!(row.device_name, "Device A");
        assert_eq!(row.asn_id, 0);
        assert_eq!(row.asn_name, "Example ASN");
        assert_eq!(row.asn_country, "US");
        assert_eq!(row.protocol_name, "HTTPS");
        assert_eq!(row.remote_ip, "198.51.100.20");
        assert_eq!(row.down_bps, 50_000_000);
        assert_eq!(row.up_bps, 15_000_000);
        assert_eq!(row.bytes_sent_down, 10_000);
        assert_eq!(row.bytes_sent_up, 20_000);
        assert_eq!(row.packets_sent_down, 100);
        assert_eq!(row.packets_sent_up, 200);
        assert_eq!(row.tcp_retransmits_down, 5);
        assert_eq!(row.tcp_retransmits_up, 4);
        assert_eq!(row.rtt_down_nanos, 12_000_000);
        assert_eq!(row.rtt_up_nanos, 34_000_000);
        assert_eq!(row.qoo_down, Some(77.0));
        assert_eq!(row.qoo_up, Some(66.0));
        assert_eq!(row.age_nanos, 5_000_000_000);
        assert!((row.opacity - (5.0 / 6.0)).abs() < f64::EPSILON);
        assert_eq!(row.sort_rate_bps, 65_000_000.0);
        assert_eq!(row.retransmit_down_pct, 0.05);
        assert_eq!(row.retransmit_up_pct, 0.02);

        let mut flow_without_device = flow;
        flow_without_device.device_name = String::new();
        flow_without_device.packets_sent = DownUpOrder::new(0, 0);
        flow_without_device.tcp_retransmits = DownUpOrder::new(0, 0);
        let row = circuit_flow_snapshot_row_from_flow(
            &flow_without_device,
            if flow_without_device.device_name.is_empty() {
                "Unknown".to_string()
            } else {
                flow_without_device.device_name.clone()
            },
            &flow_without_device.display,
            None,
            75_000_000_000,
        );
        assert_eq!(row.device_name, "Unknown");
        assert_eq!(row.retransmit_down_pct, 0.0);
        assert_eq!(row.retransmit_up_pct, 0.0);
    }
}
