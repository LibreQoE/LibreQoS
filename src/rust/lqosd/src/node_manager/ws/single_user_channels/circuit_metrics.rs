use crate::node_manager::local_api::circuit_live::{
    CircuitLiveMetrics, CircuitMetricsQuery, circuit_live_metrics,
};
use crate::node_manager::ws::messages::{WsResponse, encode_ws_message};
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::debug;

pub(super) async fn watch_circuit_metrics(
    query: CircuitMetricsQuery,
    tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_metrics: Option<Vec<CircuitLiveMetrics>> = None;

    loop {
        let metrics = circuit_live_metrics(&query);
        let changed = last_metrics.as_ref() != Some(&metrics);
        let response = if last_metrics.is_none() {
            Some(WsResponse::CircuitMetricsSnapshot {
                data: metrics.clone(),
            })
        } else if changed {
            Some(WsResponse::CircuitMetricsUpdate {
                data: metrics.clone(),
            })
        } else {
            None
        };
        last_metrics = Some(metrics);

        if let Some(response) = response {
            match encode_ws_message(&response) {
                Ok(payload) => {
                    if tx.send(payload).await.is_err() {
                        debug!("CircuitMetrics watcher channel closed");
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        ticker.tick().await;
    }
}
