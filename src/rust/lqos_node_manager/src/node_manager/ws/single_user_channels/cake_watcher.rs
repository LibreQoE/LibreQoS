use lqos_bus::{bus_request, BusRequest, QueueStoreTransit};

pub(super) async fn cake_watcher(circuit_id: String, tx: tokio::sync::mpsc::Sender<String>) {
    const INTERVAL_MS: u64 = 1000;
    let _ = bus_request(vec![BusRequest::WatchQueue(circuit_id.clone())]).await;

    let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(INTERVAL_MS));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let _ = bus_request(vec![BusRequest::WatchQueue(circuit_id.clone())]).await;
        if let Ok(replies) = bus_request(vec![BusRequest::GetRawQueueData(circuit_id.clone())]).await {

            for reply in replies.into_iter() {
                match reply {
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
    }
}