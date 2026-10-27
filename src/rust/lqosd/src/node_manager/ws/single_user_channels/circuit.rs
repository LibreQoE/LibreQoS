use crate::node_manager::local_api::circuit_activity::{CircuitSummaryData, circuit_flow_counts};
use crate::node_manager::ws::messages::{CircuitDevicesResult, WsResponse, encode_ws_message};
use crate::node_manager::ws::single_user_channels::try_send_private_payload;
use crate::node_manager::ws::ticker::all_circuits;
use crate::rtt_exclusions;
use crate::shaped_devices_tracker::{catalog_rows_to_circuits, overlay_device_rates};
use crate::throughput_tracker::{circuit_current_qoo, circuit_current_rtt_p50_nanos};
use lqos_bus::{BusRequest, Circuit};
use lqos_utils::hash_to_i64;
use lqos_utils::units::{DownUpOrder, down_up_retransmit_sample};
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::info;

fn qoo_score_for_circuit(circuit: &str) -> Option<f32> {
    let circuit_hash = circuit_hash_for_id(circuit)?;
    let qoo = circuit_current_qoo(circuit_hash);
    match (qoo.down, qoo.up) {
        (Some(d), Some(u)) => Some(d.min(u)),
        (Some(d), None) => Some(d),
        (None, Some(u)) => Some(u),
        (None, None) => None,
    }
}

fn circuit_hash_for_id(circuit: &str) -> Option<i64> {
    let trimmed = circuit.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(hash_to_i64(trimmed))
    }
}

/// T2a pure core: the circuit summary from one E2 circuit row + the
/// per-IP-overlaid device rows. Rates fold the overlaid devices
/// (shaped headline, actual companion); RTT rides the row (amendment
/// 3 — the kernel RTT buffers are zero on v3); QoO rides the bridge
/// accumulator snapshot (3a QoO port); flow counts join the snapshot's
/// active-flow IPs; ASN count is asserted zero (no bridge ASN data).
fn summarize_from_bridge_row(
    circuit: &str,
    devices: &[Circuit],
    row: &crate::bridge_analytics::translate::CircuitLiveRow,
    active_flow_ips: &fxhash::FxHashSet<std::net::IpAddr>,
) -> CircuitSummaryData {
 // 3b display semantics: the summary's customer-facing
    // `bytes_per_second` is the E2 row's DELIVERED (shaped) series; the
    // pre-shaping ingress demand is the separate `actual_bytes_per_second`
    // (`CircuitLiveRow.bytes_per_second` is that demand).
    let bytes_per_second = row.shaped_bytes_per_second;
    let actual_bytes_per_second = row.bytes_per_second;
    let tcp_retransmit_sample = down_up_retransmit_sample(
        DownUpOrder {
            down: devices
                .iter()
                .map(|device| device.tcp_retransmit_sample.down.retransmits.get())
                .sum(),
            up: devices
                .iter()
                .map(|device| device.tcp_retransmit_sample.up.retransmits.get())
                .sum(),
        },
        DownUpOrder {
            down: devices
                .iter()
                .map(|device| device.tcp_retransmit_sample.down.packets.get())
                .sum(),
            up: devices
                .iter()
                .map(|device| device.tcp_retransmit_sample.up.packets.get())
                .sum(),
        },
    );
    let active_flow_count = devices
        .iter()
        .filter(|device| active_flow_ips.contains(&device.ip))
        .count();
    // lqos-74t.47: compose the three-way legs from the two per-direction medians.
    let (subscriber_rtt_nanos, internet_rtt_nanos, full_rtt_nanos) =
        crate::shaped_devices_tracker::circuit_live::rtt_legs_nanos(row.rtt_current_p50_nanos);
    CircuitSummaryData {
        circuit_id: circuit.to_string(),
        bytes_per_second,
        actual_bytes_per_second,
        rtt_current_p50_nanos: row.rtt_current_p50_nanos,
        subscriber_rtt_nanos,
        internet_rtt_nanos,
        full_rtt_nanos,
        tcp_retransmit_sample,
        qoo_score: qoo_score_for_circuit(circuit),
        rtt_excluded: rtt_exclusions::is_excluded_circuit_id(circuit),
        active_flow_count,
        // No ASN attribution exists on the bridge snapshot — asserted
        // zero, never fabricated.
        active_asn_count: 0,
    }
}

fn summarize_circuit_devices(circuit: &str, devices: &[Circuit]) -> CircuitSummaryData {
    // T2a (t2_live_surface_repoint_derivation): on the bridge path the
    // devices rows already carry the E2 per-IP rates (overlaid in
    // `circuit_devices_snapshot`); RTT comes from the E2 circuit row —
    // NOT the flowbee/kernel-fed CIRCUIT_RTT_BUFFERS (zero on v3) —
    // and flow counts join the snapshot's active-flow IPs. QoO has no
    // bridge source (E2 §5.1.3 pilot scope): None, never fabricated.
    // All-or-nothing per field source: no kernel+bridge blending.
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        if let Some(row) = snapshot.by_circuit_id.get(circuit) {
            return summarize_from_bridge_row(
                circuit,
                devices,
                row,
                &snapshot.active_flow_ips,
            );
        }
    }

    let bytes_per_second = devices
        .iter()
        .fold(DownUpOrder::default(), |mut acc, device| {
            acc.down += device.bytes_per_second.down;
            acc.up += device.bytes_per_second.up;
            acc
        });

    let actual_bytes_per_second = devices
        .iter()
        .fold(DownUpOrder::default(), |mut acc, device| {
            acc.down += device.actual_bytes_per_second.down;
            acc.up += device.actual_bytes_per_second.up;
            acc
        });

    let tcp_retransmit_sample = down_up_retransmit_sample(
        DownUpOrder {
            down: devices
                .iter()
                .map(|device| device.tcp_retransmit_sample.down.retransmits.get())
                .sum(),
            up: devices
                .iter()
                .map(|device| device.tcp_retransmit_sample.up.retransmits.get())
                .sum(),
        },
        DownUpOrder {
            down: devices
                .iter()
                .map(|device| device.tcp_retransmit_sample.down.packets.get())
                .sum(),
            up: devices
                .iter()
                .map(|device| device.tcp_retransmit_sample.up.packets.get())
                .sum(),
        },
    );

    let rtt_current_p50_nanos = circuit_hash_for_id(circuit)
        .map(circuit_current_rtt_p50_nanos)
        .unwrap_or_default();

    let (active_flow_count, active_asn_count) = circuit_flow_counts(circuit);
    // lqos-74t.47: compose the three-way legs from the two per-direction medians.
    let (subscriber_rtt_nanos, internet_rtt_nanos, full_rtt_nanos) =
        crate::shaped_devices_tracker::circuit_live::rtt_legs_nanos(rtt_current_p50_nanos);

    CircuitSummaryData {
        circuit_id: circuit.to_string(),
        bytes_per_second,
        actual_bytes_per_second,
        rtt_current_p50_nanos,
        subscriber_rtt_nanos,
        internet_rtt_nanos,
        full_rtt_nanos,
        tcp_retransmit_sample,
        qoo_score: qoo_score_for_circuit(circuit),
        rtt_excluded: rtt_exclusions::is_excluded_circuit_id(circuit),
        active_flow_count,
        active_asn_count,
    }
}

pub async fn circuit_devices_snapshot(
    circuit: &str,
    bus_tx: tokio::sync::mpsc::Sender<(
        tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
        BusRequest,
    )>,
) -> Vec<Circuit> {
 // T2a correction (driver live-demo finding, the kernel
    // device list is EMPTY on the v3 bridge path — `snapshot_circuits`
    // iterates `THROUGHPUT_TRACKER.raw_data` (§5.3: kernel XDP, empty
    // by construction), so the old all_circuits+overlay shape left
    // the circuit page's rates arm folding over ZERO rows while RTT
    // (read straight from the E2 row) stayed live. On the active path
    // the identity rows come from the CATALOG and the rates from the
    // E2 by_ip join. All-or-nothing per the T2 pin: the legacy kernel
    // path is untouched when the analytics actor is inactive.
    if crate::bridge_analytics::is_active() {
        let snapshot = crate::bridge_analytics::snapshot();
        let catalog = lqos_network_devices::shaped_devices_catalog();
        let catalog_devices = catalog.devices_for_circuit_id(circuit);
        let mut devices = catalog_rows_to_circuits(&catalog_devices);
        overlay_device_rates(&mut devices, &snapshot.by_ip);
        return devices;
    }
    all_circuits(bus_tx)
        .await
        .into_iter()
        .filter(|device| device.circuit_id.as_deref() == Some(circuit))
        .collect()
}

pub(super) async fn circuit_watcher(
    circuit: String,
    tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
    bus_tx: tokio::sync::mpsc::Sender<(
        tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
        BusRequest,
    )>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let devices_for_circuit = circuit_devices_snapshot(&circuit, bus_tx.clone()).await;
        let response = WsResponse::CircuitWatcher {
            data: summarize_circuit_devices(&circuit, &devices_for_circuit),
        };

        if let Ok(payload) = encode_ws_message(&response) {
            if !try_send_private_payload(&tx, payload, "CircuitWatcher") {
                info!("CircuitWatcher channel is gone");
                break;
            }
        } else {
            info!("CircuitWatcher encode failed");
            break;
        }
    }
}

pub fn circuit_devices_result(circuit: String, devices: Vec<Circuit>) -> WsResponse {
    WsResponse::CircuitDevicesResult {
        data: CircuitDevicesResult {
            circuit_id: circuit,
            devices,
            ok: true,
        },
    }
}

#[cfg(test)]
mod t2a_tests {
    use super::*;
    use crate::bridge_analytics::IpLiveRow;
    use fxhash::FxHashSet;
    use std::net::IpAddr;

    fn device(ip: IpAddr) -> Circuit {
        Circuit {
            ip,
            bytes_per_second: DownUpOrder::zeroed(),
            actual_bytes_per_second: DownUpOrder::zeroed(),
            median_latency: None,
            rtt_current_p50_nanos: DownUpOrder {
                down: None,
                up: None,
            },
            rtt_current_p95_nanos: DownUpOrder {
                down: None,
                up: None,
            },
            qoo: DownUpOrder { down: None, up: None },
            rtt_total_p50_nanos: DownUpOrder {
                down: None,
                up: None,
            },
            rtt_total_p95_nanos: DownUpOrder {
                down: None,
                up: None,
            },
            tcp_retransmit_sample: down_up_retransmit_sample(
                DownUpOrder { down: 0, up: 0 },
                DownUpOrder { down: 0, up: 0 },
            ),
            circuit_id: Some("c1".to_string()),
            device_id: Some("d1".to_string()),
            parent_node: Some("ap".to_string()),
            circuit_name: Some("c1".to_string()),
            device_name: Some("dev1".to_string()),
            plan: DownUpOrder { down: 0.0, up: 0.0 },
            last_seen_nanos: 0,
        }
    }

    #[test]
    fn device_overlay_maps_by_ip_and_leaves_unknown_ips_honest_zero() {
        let ip_a: IpAddr = "100.97.0.1".parse().expect("test invariant");
        let ip_b: IpAddr = "100.97.0.2".parse().expect("test invariant");
        let mut by_ip = FxHashSet::default();
        by_ip.insert(ip_a); // placeholder to exercise the set type
        let mut by_ip_map = fxhash::FxHashMap::default();
        by_ip_map.insert(
            ip_a,
            IpLiveRow {
                bytes_per_second: DownUpOrder { down: 1000, up: 500 },
                shaped_bytes_per_second: DownUpOrder { down: 900, up: 450 },
                tcp_packets: 100,
                tcp_retransmits: 3,
                packets_per_second: DownUpOrder::zeroed(),
                packets_sent_per_second: DownUpOrder::zeroed(),
                rtt_current_p50_nanos: None,
                last_seen_nanos: 0,
                total_bytes: DownUpOrder::zeroed(),
            },
        );

        let mut devices = vec![device(ip_a), device(ip_b)];
        overlay_device_rates(&mut devices, &by_ip_map);

        // Matched device: shaped headline + actual companion + sample.
        assert_eq!(devices[0].bytes_per_second.down, 900);
        assert_eq!(devices[0].actual_bytes_per_second.down, 1000);
        assert_eq!(devices[0].tcp_retransmit_sample.down.retransmits.get(), 3);
        assert_eq!(devices[0].tcp_retransmit_sample.down.packets.get(), 100);
        // Unmatched device: honest zeros, untouched identity.
        assert_eq!(devices[1].bytes_per_second.down, 0);
        assert_eq!(devices[1].actual_bytes_per_second.up, 0);
    }

    #[test]
    fn summary_from_bridge_row_uses_the_row_delivered_series() {
        let ip_a: IpAddr = "100.97.0.1".parse().expect("test invariant");
        let ip_b: IpAddr = "100.97.0.2".parse().expect("test invariant");
        let mut by_ip_map = fxhash::FxHashMap::default();
        by_ip_map.insert(
            ip_a,
            IpLiveRow {
                bytes_per_second: DownUpOrder { down: 1000, up: 500 },
                shaped_bytes_per_second: DownUpOrder { down: 900, up: 450 },
                tcp_packets: 100,
                tcp_retransmits: 3,
                packets_per_second: DownUpOrder::zeroed(),
                packets_sent_per_second: DownUpOrder::zeroed(),
                rtt_current_p50_nanos: None,
                last_seen_nanos: 0,
                total_bytes: DownUpOrder::zeroed(),
            },
        );
        by_ip_map.insert(
            ip_b,
            IpLiveRow {
                bytes_per_second: DownUpOrder { down: 400, up: 200 },
                shaped_bytes_per_second: DownUpOrder { down: 350, up: 175 },
                tcp_packets: 40,
                tcp_retransmits: 1,
                packets_per_second: DownUpOrder::zeroed(),
                packets_sent_per_second: DownUpOrder::zeroed(),
                rtt_current_p50_nanos: None,
                last_seen_nanos: 0,
                total_bytes: DownUpOrder::zeroed(),
            },
        );
        let mut devices = vec![device(ip_a), device(ip_b)];
        overlay_device_rates(&mut devices, &by_ip_map);

        let row = crate::bridge_analytics::translate::CircuitLiveRow {
 // 3b: `bytes_per_second` is the enqueued DEMAND,
            // `shaped_bytes_per_second` the DELIVERED series.
            bytes_per_second: DownUpOrder { down: 1400, up: 700 },
            shaped_bytes_per_second: DownUpOrder { down: 1250, up: 625 },
            rtt_current_p50_nanos: DownUpOrder {
                down: Some(2_807_000),
                up: Some(2_807_000),
            },
            rtt_histogram: Default::default(),
            tcp_retransmit_sample: Default::default(),
            last_seen_nanos: 1_000_000,
            bridge_circuit_id: 1,
        };
        let mut active_flows = FxHashSet::default();
        active_flows.insert(ip_a);

        let summary = summarize_from_bridge_row("c1", &devices, &row, &active_flows);
        // Headline = the ROW's DELIVERED (shaped) series — the customer value.
        assert_eq!(summary.bytes_per_second.down, 1250);
        assert_eq!(summary.bytes_per_second.up, 625);
        // Companion = the ROW's enqueued DEMAND series.
        assert_eq!(summary.actual_bytes_per_second.down, 1400);
        assert_eq!(summary.actual_bytes_per_second.up, 700);
        // RTT rides the ROW (amendment 3), not the kernel buffers.
        assert_eq!(summary.rtt_current_p50_nanos.down, Some(2_807_000));
        // Retransmits folded from the device samples (3+1, 100+40).
        assert_eq!(summary.tcp_retransmit_sample.down.retransmits.get(), 4);
        assert_eq!(summary.tcp_retransmit_sample.down.packets.get(), 140);
        // Flow count joins active-flow IPs; QoO/ASN asserted zero.
        assert_eq!(summary.active_flow_count, 1);
        assert_eq!(summary.active_asn_count, 0);
        assert_eq!(summary.qoo_score, None);
        assert_eq!(summary.circuit_id, "c1");
    }
}

