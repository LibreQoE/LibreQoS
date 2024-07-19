use lqos_bus::QueueStoreTransit;
use lqos_queue_tracker::{add_watched_queue, get_raw_circuit_data, still_watching};

pub(super) async fn cake_watcher(circuit_id: String, tx: tokio::sync::mpsc::Sender<String>) {
    const INTERVAL_MS: u64 = 1000;
    add_watched_queue(&circuit_id);

    let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(INTERVAL_MS));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        still_watching(&circuit_id);

        match get_raw_circuit_data(&circuit_id) {
            lqos_bus::BusResponse::RawQueueData(Some(msg)) => {
                if let Ok(json) = serde_json::to_string(&QueueStoreTransit::from(*msg)) {
                    let send_result = tx.send(json.to_string()).await;
                    if send_result.is_err() {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}