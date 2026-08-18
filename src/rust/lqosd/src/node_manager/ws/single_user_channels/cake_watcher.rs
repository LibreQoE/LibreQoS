use crate::node_manager::ws::messages::{WsResponse, encode_ws_message};
use crate::node_manager::ws::single_user_channels::try_send_private_payload;
use lqos_queue_tracker::{add_watched_queue, get_raw_circuit_data, still_watching};

pub(super) async fn cake_watcher(
    circuit_id: String,
    tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
) {
    const INTERVAL_MS: u64 = 1000;
    add_watched_queue(&circuit_id);

    let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(INTERVAL_MS));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        still_watching(&circuit_id);

        if let lqos_bus::BusResponse::RawQueueData(Some(msg)) = get_raw_circuit_data(&circuit_id) {
            let response = WsResponse::CakeWatcher { data: *msg };
            if let Ok(payload) = encode_ws_message(&response)
                && !try_send_private_payload(&tx, payload, "CakeWatcher")
            {
                break;
            }
        }
    }
}
