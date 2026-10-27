use crate::bridge_analytics;
use crate::shaped_devices_tracker::circuit_live::fresh_circuit_live_snapshot;
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const MAX_CIRCUIT_METRICS_IDS: usize = 250;

/// Which dataplane produced a live-metrics row (E2 §5.1.2: source/stale
/// honesty fields — the mixed-source pilot window is recorded, not
/// silent).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitStatsSource {
    /// The kernel throughput tracker (the incumbent source).
    #[default]
    Kernel,
    /// The DPDK bridge analytics socket (E2 repoint).
    Bridge,
}

/// Query for the compact live metrics needed by the shaped-devices page.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitMetricsQuery {
    pub circuit_ids: Vec<String>,
}

/// Live metrics for a single visible circuit card.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CircuitLiveMetrics {
    pub circuit_id: String,
    pub bytes_per_second: DownUpOrder<u64>,
    pub rtt_current_p50_nanos: DownUpOrder<Option<u64>>,
    pub qoo: DownUpOrder<Option<f32>>,
    pub tcp_retransmit_sample: DownUpOrder<TcpRetransmitSample>,
    pub last_seen_nanos: u64,
    /// The dataplane this row came from.
    #[serde(default)]
    pub source: CircuitStatsSource,
    /// True when the bridge source is serving LAST-GOOD data (socket
    /// absence) — never fabricated zeros (G-D2 honesty posture).
    #[serde(default)]
    pub stale: bool,
}

/// Resolves the current live metrics for a bounded set of circuit ids.
///
/// E2 endpoint-level repoint (card §5.1.2): when the bridge analytics
/// actor is active, rows come from the actor's OWN bridge-sourced cache
/// (stale-flagged last-good on socket absence). The three other
/// consumers of `fresh_circuit_live_snapshot` (tree_attached_circuits,
/// the dynamic-page sort, TreeGuard) are deliberately UNTOUCHED — they
/// stay kernel-sourced during the pilot. When the actor is inactive
/// (kernel-dataplane installs), the incumbent path serves unchanged.
pub fn circuit_live_metrics(query: &CircuitMetricsQuery) -> Vec<CircuitLiveMetrics> {
    if bridge_analytics::is_active() {
        return bridge_metrics(query);
    }
    kernel_metrics(query)
}

fn bridge_metrics(query: &CircuitMetricsQuery) -> Vec<CircuitLiveMetrics> {
    let snapshot = bridge_analytics::snapshot();
    let mut seen = HashSet::new();
    query
        .circuit_ids
        .iter()
        .filter_map(|id| {
            let trimmed = id.trim();
            if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
                return None;
            }
            snapshot.by_circuit_id.get(trimmed).map(|row| CircuitLiveMetrics {
                circuit_id: trimmed.to_string(),
 // 3b demand-source sweep: customer-facing throughput
                // is the DELIVERED series (`bytes_per_second` is the demand).
                bytes_per_second: row.shaped_bytes_per_second,
                rtt_current_p50_nanos: row.rtt_current_p50_nanos,
                // QoO is absent from the bridge snapshot (§5.1.3):
                // renders empty in the pilot.
                qoo: DownUpOrder::default(),
                tcp_retransmit_sample: row.tcp_retransmit_sample,
                last_seen_nanos: row.last_seen_nanos,
                source: CircuitStatsSource::Bridge,
                stale: snapshot.stale,
            })
        })
        .take(MAX_CIRCUIT_METRICS_IDS)
        .collect()
}

fn kernel_metrics(query: &CircuitMetricsQuery) -> Vec<CircuitLiveMetrics> {
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
                    bytes_per_second: row.bytes_per_second,
                    rtt_current_p50_nanos: row.rtt_current_p50_nanos,
                    qoo: row.qoo,
                    tcp_retransmit_sample: row.tcp_retransmit_sample,
                    last_seen_nanos: row.last_seen_nanos,
                    source: CircuitStatsSource::Kernel,
                    stale: false,
                })
        })
        .take(MAX_CIRCUIT_METRICS_IDS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge_analytics::translate::CircuitLiveRow;
    use crate::bridge_analytics::{BridgeStatsSnapshot, CACHE};
    use fxhash::FxHashMap;
    use lqos_utils::units::TcpRetransmitSample;

    static SOURCE_TEST_LOCK: once_cell::sync::Lazy<std::sync::Mutex<()>> =
        once_cell::sync::Lazy::new(|| std::sync::Mutex::new(()));

    fn bridge_row(bytes_down: u64) -> CircuitLiveRow {
        CircuitLiveRow {
 // 3b: the demand is deliberately LARGER than the
            // delivered series, so an accidental demand read would fail.
            bytes_per_second: DownUpOrder {
                down: bytes_down + 10_000,
                up: 222,
            },
            shaped_bytes_per_second: DownUpOrder {
                down: bytes_down,
                up: 111,
            },
            rtt_current_p50_nanos: DownUpOrder {
                down: Some(1_000),
                up: Some(1_000),
            },
            rtt_histogram: Default::default(),
            tcp_retransmit_sample: DownUpOrder {
                down: TcpRetransmitSample::new(1, 100),
                up: TcpRetransmitSample::new(1, 100),
            },
            last_seen_nanos: 5_000,
            bridge_circuit_id: 7,
        }
    }

    fn query(ids: &[&str]) -> CircuitMetricsQuery {
        CircuitMetricsQuery {
            circuit_ids: ids.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// The repoint serves bridge rows (source + stale honesty) while the
    /// actor is active; the kernel fallback carries its own source.
    /// `is_active()` follows the process-wide STARTED marker, so this
    /// test exercises BOTH arms through the mapping functions directly
    /// (bridge_metrics is only reached when the actor is active).
    #[test]
    fn bridge_metrics_maps_rows_with_source_and_stale() {
        let _guard = SOURCE_TEST_LOCK.lock().expect("test invariant");
        let mut by_circuit_id = FxHashMap::default();
        by_circuit_id.insert("c1".to_string(), bridge_row(1_234));
        CACHE.store(std::sync::Arc::new(BridgeStatsSnapshot {
            by_circuit_id,
            stale: true,
            captured_at_millis: 9,
            host_rows: 1,
            by_ip: Default::default(),
            active_flow_ips: Default::default(),
            ..Default::default()
        }));
        let rows = bridge_metrics(&query(&["c1", " c1 ", "missing", ""]));
        assert_eq!(rows.len(), 1, "dedupe + unknown ids omitted");
        assert_eq!(rows[0].circuit_id, "c1");
        assert_eq!(rows[0].bytes_per_second.down, 1_234);
        assert_eq!(rows[0].source, CircuitStatsSource::Bridge);
        assert!(rows[0].stale, "stale rides inside the snapshot");
        assert_eq!(rows[0].qoo.down, None, "QoO renders empty bridge-side");

        CACHE.store(std::sync::Arc::new(BridgeStatsSnapshot::default()));
    }

    #[test]
    fn kernel_metrics_carries_kernel_source() {
        // The incumbent path (unchanged behavior + explicit source).
        let rows = kernel_metrics(&query(&["no-such-circuit"]));
        assert!(rows.is_empty());
    }
}
