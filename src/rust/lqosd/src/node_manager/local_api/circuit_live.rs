use crate::shaped_devices_tracker::circuit_live::fresh_circuit_live_snapshot;
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const MAX_CIRCUIT_METRICS_IDS: usize = 250;

/// Query for the compact live metrics needed by the shaped-devices page.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitMetricsQuery {
    pub circuit_ids: Vec<String>,
}

/// Live metrics for a single visible circuit card.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitLiveMetrics {
    pub circuit_id: String,
    pub enqueue_bytes_per_second: DownUpOrder<u64>,
    #[serde(default)]
    pub xmit_bytes_per_second: DownUpOrder<u64>,
    pub rtt_current_p50_nanos: DownUpOrder<Option<u64>>,
    pub qoo: DownUpOrder<Option<f32>>,
    pub tcp_retransmit_sample: DownUpOrder<TcpRetransmitSample>,
    pub last_seen_nanos: u64,
}

/// Resolves the current live metrics for a bounded set of circuit ids.
pub fn circuit_live_metrics(query: &CircuitMetricsQuery) -> Vec<CircuitLiveMetrics> {
    let snapshot = fresh_circuit_live_snapshot();
    let mut seen = HashSet::new();
    query
        .circuit_ids
        .iter()
        .filter_map(|id| {
            let trimmed = id.trim();
            if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
                return None;
            }
            snapshot
                .by_circuit_id
                .get(trimmed)
                .map(|row| CircuitLiveMetrics {
                    circuit_id: row.circuit_id.clone(),
                    enqueue_bytes_per_second: row.enqueue_bytes_per_second,
                    xmit_bytes_per_second: row.xmit_bytes_per_second,
                    rtt_current_p50_nanos: row.rtt_current_p50_nanos,
                    qoo: row.qoo,
                    tcp_retransmit_sample: row.tcp_retransmit_sample,
                    last_seen_nanos: row.last_seen_nanos,
                })
        })
        .take(MAX_CIRCUIT_METRICS_IDS)
        .collect()
}
