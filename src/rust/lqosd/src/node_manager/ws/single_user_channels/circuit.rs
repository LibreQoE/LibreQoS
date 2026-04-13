use crate::node_manager::local_api::circuit_activity::{CircuitSummaryData, circuit_flow_counts};
use crate::node_manager::ws::messages::{CircuitDevicesResult, WsResponse, encode_ws_message};
use crate::node_manager::ws::ticker::all_circuits;
use crate::rtt_exclusions;
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

fn summarize_circuit_devices(circuit: &str, devices: &[Circuit]) -> CircuitSummaryData {
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

    CircuitSummaryData {
        circuit_id: circuit.to_string(),
        bytes_per_second,
        actual_bytes_per_second,
        rtt_current_p50_nanos,
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
            if tx.send(payload).await.is_err() {
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
