use std::time::Duration;
use serde::Serialize;
use tokio::time::MissedTickBehavior;
use crate::node_manager::ws::ticker::{all_circuits, Circuit};

#[derive(Serialize)]
pub struct Devices {
    pub circuit_id: String,
    pub devices: Vec<Circuit>,
}

pub(super) async fn circuit_watcher(circuit: String, tx: tokio::sync::mpsc::Sender<String>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;

        // Get all circuits and filter them
        let devices_for_circuit: Vec<_> = all_circuits()
            .into_iter()
            .filter(|c| {
                if let Some(c) = c.circuit_id.as_ref() {
                    *c == circuit
                } else {
                    false
                }
            })
            .collect();

        let result = Devices {
            circuit_id: circuit.clone(),
            devices: devices_for_circuit,
        };

        let message = serde_json::to_string(&result).unwrap();
        if let Err(_) = tx.send(message.to_string()).await {
            log::info!("Channel is gone");
            break;
        }
    }
}