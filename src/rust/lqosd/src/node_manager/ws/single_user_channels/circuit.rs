use crate::node_manager::ws::messages::{WsResponse, encode_ws_message};
use crate::node_manager::ws::ticker::all_circuits;
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::throughput_tracker::THROUGHPUT_TRACKER;
use lqos_bus::BusRequest;
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::info;

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

        // Get all circuits and filter them
        let devices_for_circuit: Vec<_> = all_circuits(bus_tx.clone())
            .await
            .into_iter()
            .filter(|c| {
                if let Some(c) = c.circuit_id.as_ref() {
                    *c == circuit
                } else {
                    false
                }
            })
            .collect();

        let qoo_score = {
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
        };

        let result = WsResponse::CircuitWatcher {
            circuit_id: circuit.clone(),
            devices: devices_for_circuit,
            qoo_score,
        };

        if let Ok(payload) = encode_ws_message(&result) {
            if let Err(_) = tx.send(payload).await {
                info!("Channel is gone");
                break;
            }
        } else {
            info!("CircuitWatcher encode failed");
            break;
        }
    }
}
