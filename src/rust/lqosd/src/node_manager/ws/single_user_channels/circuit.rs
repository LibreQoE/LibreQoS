use crate::node_manager::ws::messages::{WsResponse, encode_ws_message};
use crate::node_manager::ws::ticker::all_circuits;
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

        let result = WsResponse::CircuitWatcher {
            circuit_id: circuit.clone(),
            devices: devices_for_circuit,
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
