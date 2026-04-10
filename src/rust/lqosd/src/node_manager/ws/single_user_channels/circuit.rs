use crate::node_manager::local_api::circuit_activity::{CircuitSummaryData, circuit_flow_counts};
use crate::node_manager::ws::messages::{CircuitDevicesResult, WsResponse, encode_ws_message};
use crate::node_manager::ws::ticker::all_circuits;
use crate::rtt_exclusions;
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::throughput_tracker::THROUGHPUT_TRACKER;
use lqos_bus::{BusRequest, Circuit};
use lqos_utils::units::{DownUpOrder, down_up_retransmit_sample};
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::info;

const QUEUING_ACTIVITY_RTT_FLOOR_BPS: u64 = 200_000;

fn qoo_score_for_circuit(circuit: &str) -> Option<f32> {
    let shaped = SHAPED_DEVICES.load();
    let circuit_hash = shaped
        .devices
        .iter()
        .find(|d| d.circuit_id == circuit)
        .map(|d| d.circuit_hash);
    circuit_hash.and_then(|hash| {
        let qoq_heatmaps = THROUGHPUT_TRACKER.circuit_qoq_heatmaps.lock();
        qoq_heatmaps.get(&hash).and_then(|heatmap| {
            let blocks = heatmap.blocks();
            let dl = blocks.download_total.last().copied().flatten();
            let ul = blocks.upload_total.last().copied().flatten();
            match (dl, ul) {
                (Some(d), Some(u)) => Some(d.min(u)),
                (Some(d), None) => Some(d),
                (None, Some(u)) => Some(u),
                (None, None) => None,
            }
        })
    })
}

fn weighted_directional_rtt_p50_nanos(
    devices: &[Circuit],
    direction: fn(&Circuit) -> (u64, Option<u64>),
) -> Option<u64> {
    let mut weighted_entries = Vec::new();
    let mut fallback_values = Vec::new();

    for device in devices {
        let (throughput_bps, rtt_nanos_opt) = direction(device);
        let Some(rtt_nanos) = rtt_nanos_opt else {
            continue;
        };
        fallback_values.push(rtt_nanos);
        if throughput_bps > QUEUING_ACTIVITY_RTT_FLOOR_BPS {
            weighted_entries.push((rtt_nanos, throughput_bps));
        }
    }

    if !weighted_entries.is_empty() {
        weighted_entries.sort_by_key(|(rtt, _)| *rtt);
        let total_weight: u128 = weighted_entries
            .iter()
            .map(|(_, weight)| *weight as u128)
            .sum();
        let threshold = total_weight / 2;
        let mut running = 0_u128;
        for (rtt, weight) in weighted_entries {
            running += weight as u128;
            if running >= threshold {
                return Some(rtt);
            }
        }
    }

    if fallback_values.is_empty() {
        return None;
    }
    fallback_values.sort_unstable();
    let middle = fallback_values.len() / 2;
    if fallback_values.len() % 2 == 1 {
        Some(fallback_values[middle])
    } else {
        Some((fallback_values[middle - 1] + fallback_values[middle]) / 2)
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

    let rtt_current_p50_nanos = DownUpOrder {
        down: weighted_directional_rtt_p50_nanos(devices, |device| {
            (
                device.bytes_per_second.down.saturating_mul(8),
                device.rtt_current_p50_nanos.down,
            )
        }),
        up: weighted_directional_rtt_p50_nanos(devices, |device| {
            (
                device.bytes_per_second.up.saturating_mul(8),
                device.rtt_current_p50_nanos.up,
            )
        }),
    };

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
