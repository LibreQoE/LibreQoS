use lqos_bus::QueueStoreTransit;
use lqos_config::load_config;
use lqos_queue_tracker::{add_watched_queue, get_raw_circuit_data, still_watching};

pub(super) async fn cake_watcher(circuit_id: String, tx: tokio::sync::mpsc::Sender<String>) {
    let interval_ms = if let Ok(config) = load_config() {
        config.queue_check_period_ms
    } else {
        0
    };
    add_watched_queue(&circuit_id);

    let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(interval_ms));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        still_watching(&circuit_id);

        match get_raw_circuit_data(&circuit_id) {
            lqos_bus::BusResponse::RawQueueData(Some(msg)) => {
                let json = serde_json::to_string(&QueueStoreTransit::from(*msg)).unwrap();
                let send_result = tx.send(json.to_string()).await;
                if send_result.is_err() {
                    break;
                }
            }
            _ => {}
        }
    }
}