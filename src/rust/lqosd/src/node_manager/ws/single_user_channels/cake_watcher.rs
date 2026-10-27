//! Circuit CAKE/qdisc watcher.
//!
//! K3-B: the kernel `lqos_queue_tracker` raw-queue feed is retired. The
//! bridge (`RequestKind::QueueStats`) subscription to the bridge analytics
//! socket is the only source, mapped into the `QueueStoreTransit` UI payload.
//! The bridge wire carries per-DIRECTION drops/marks and per-tin only
//! `{sent_bytes, backlog_bytes, sojourn_p99_ns}` — so per-tin sent/backlog/sojourn map directly (cumulative sent_bytes is
//! differenced per tick), the delay graph is relabelled to the measured p99
//! sojourn, and the marks/drops graphs render the direction aggregate with
//! `per_direction = true` (never lying per-tin zeros).

use crate::bridge_analytics;
use crate::node_manager::ws::messages::{WsResponse, encode_ws_message};
use crate::node_manager::ws::single_user_channels::try_send_private_payload;
use analytics_ipc::QdiscDirectionRow;
use lqos_bus::{CakeDiffTinTransit, CakeDiffTransit, CakeTransit, QueueStoreTransit};
use std::collections::VecDeque;

const INTERVAL_MS: u64 = 1000;
/// The UI history window (matches `CAKE_CHART_WINDOW_SECONDS`).
const HISTORY_SAMPLES: usize = 180;

pub(super) async fn cake_watcher(
    circuit_id: String,
    tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
) {
    bridge_cake_watcher(circuit_id, tx).await;
}

async fn bridge_cake_watcher(
    circuit_id: String,
    tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
) {
    let mut history: VecDeque<(CakeDiffTransit, CakeDiffTransit)> =
        VecDeque::with_capacity(HISTORY_SAMPLES);
    let mut previous: Option<[DirectionSeed; 2]> = None;

    let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(INTERVAL_MS));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        // Unknown circuit (not yet in the actor's by-circuit cache) or a
        // failed request: skip this tick rather than render a fabricated row.
        let Some(row) = bridge_analytics::queue_stats_for_circuit(&circuit_id) else {
            continue;
        };
        let (down, up, seed) = map_direction_pair(
            &row.download,
            &row.upload,
            previous.as_ref(),
        );
        previous = Some(seed);
        if history.len() == HISTORY_SAMPLES {
            history.pop_front();
        }
        history.push_back((down, up));

        let kind = if row.download.tins.is_empty() {
            "none"
        } else {
            "cake"
        };
        let response = WsResponse::CakeWatcher {
            data: QueueStoreTransit {
                history: history.iter().cloned().collect(),
                history_head: 0,
                current_download: CakeTransit {
                    memory_used: row.download.memory_used,
                },
                current_upload: CakeTransit {
                    memory_used: row.upload.memory_used,
                },
                kind_down: kind.to_string(),
                kind_up: kind.to_string(),
                per_direction: true,
                // `.115`: the WS ring path is not the bus snapshot path.
                ..QueueStoreTransit::default()
            },
        };
        if let Ok(payload) = encode_ws_message(&response)
            && !try_send_private_payload(&tx, payload, "CakeWatcher")
        {
            break;
        }
    }
}

/// Cumulative per-direction seeds for interval differencing. `sent_bytes` is
/// cumulative on the bridge wire; `backlog_bytes`/`sojourn_p99_ns`/queue depth
/// are instantaneous; `drop_pkts`/`aqm_marks` are cumulative direction totals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DirectionSeed {
    tins_sent_bytes: Vec<u64>,
    drops: u64,
    marks: u64,
}

/// Maps one direction's bridge row + the previous seed into the UI diff and
/// the next seed. First sight yields zero deltas (the bootstrap rule).
fn direction_diff(
    current: &QdiscDirectionRow,
    previous: Option<&DirectionSeed>,
) -> (CakeDiffTransit, DirectionSeed) {
    let mut tins = Vec::with_capacity(current.tins.len());
    let mut seed_tins = Vec::with_capacity(current.tins.len());
    let mut bytes = 0u64;
    for (index, tin) in current.tins.iter().enumerate() {
        let before = previous
            .and_then(|seed| seed.tins_sent_bytes.get(index).copied())
            .unwrap_or(tin.sent_bytes);
        let delta = tin.sent_bytes.saturating_sub(before);
        bytes = bytes.saturating_add(delta);
        tins.push(CakeDiffTinTransit {
            sent_bytes: delta,
            backlog_bytes: tin.backlog_bytes,
            base_delay_us: tin.sojourn_p99_ns / 1_000,
            ..CakeDiffTinTransit::default()
        });
        seed_tins.push(tin.sent_bytes);
    }
    let drops = previous
        .map(|seed| current.drop_pkts.saturating_sub(seed.drops))
        .unwrap_or(0);
    let marks = previous
        .map(|seed| current.aqm_marks.saturating_sub(seed.marks))
        .unwrap_or(0);
    (
        CakeDiffTransit {
            bytes,
            packets: 0,
            qlen: current.queue_depth_pkts,
            tins,
            drops,
            marks,
        },
        DirectionSeed {
            tins_sent_bytes: seed_tins,
            drops: current.drop_pkts,
            marks: current.aqm_marks,
        },
    )
}

fn map_direction_pair(
    download: &QdiscDirectionRow,
    upload: &QdiscDirectionRow,
    previous: Option<&[DirectionSeed; 2]>,
) -> (CakeDiffTransit, CakeDiffTransit, [DirectionSeed; 2]) {
    let (down, down_seed) = direction_diff(download, previous.map(|seed| &seed[0]));
    let (up, up_seed) = direction_diff(upload, previous.map(|seed| &seed[1]));
    (down, up, [down_seed, up_seed])
}

#[cfg(test)]
mod tests {
    use super::*;
    use analytics_ipc::QdiscTinRow;

    fn row(tins: Vec<(u64, u64, u64)>, depth: u64, drops: u64, marks: u64, mem: u64) -> QdiscDirectionRow {
        QdiscDirectionRow {
            tins: tins
                .into_iter()
                .map(|(sent, backlog, p99)| QdiscTinRow {
                    sent_bytes: sent,
                    backlog_bytes: backlog,
                    sojourn_p99_ns: p99,
                })
                .collect(),
            memory_used: mem,
            queue_depth_pkts: depth,
            drop_pkts: drops,
            drop_pkts_queue_full: 0,
            aqm_marks: marks,
        }
    }

    #[test]
    fn first_tick_seeds_without_rate_then_differences_sent_bytes() {
        let first = row(vec![(1_000, 500, 131_071)], 7, 4, 2, 1_234);
        let (diff, seed) = direction_diff(&first, None);
        assert_eq!(diff.tins[0].sent_bytes, 0, "first sight: no interval");
        assert_eq!(diff.drops, 0);
        assert_eq!(diff.marks, 0);
        // Instantaneous fields ride the first sample.
        assert_eq!(diff.tins[0].backlog_bytes, 500);
        assert_eq!(diff.tins[0].base_delay_us, 131, "sojourn 131071ns -> 131us");
        assert_eq!(diff.qlen, 7);

        let second = row(vec![(1_750, 900, 200_000)], 3, 5, 4, 1_111);
        let (diff2, _seed2) = direction_diff(&second, Some(&seed));
        assert_eq!(diff2.tins[0].sent_bytes, 750);
        assert_eq!(diff2.bytes, 750);
        assert_eq!(diff2.qlen, 3);
        assert_eq!(diff2.tins[0].backlog_bytes, 900);
        assert_eq!(diff2.tins[0].base_delay_us, 200);
        assert_eq!(diff2.drops, 1);
        assert_eq!(diff2.marks, 2);
    }

    #[test]
    fn per_tin_marks_and_drops_are_zero_bridge_side() {
        // The bridge has no per-tin drops/marks; the direction totals live on
        // the diff and the UI renders them via per_direction.
        let first = row(vec![(10, 1, 0), (20, 2, 0)], 0, 9, 3, 0);
        let (_diff, seed) = direction_diff(&first, None);
        let second = row(vec![(15, 1, 0), (30, 2, 0)], 0, 11, 8, 0);
        let (diff2, _) = direction_diff(&second, Some(&seed));
        assert_eq!(diff2.drops, 2);
        assert_eq!(diff2.marks, 5);
        for tin in &diff2.tins {
            assert_eq!(tin.drops, 0);
            assert_eq!(tin.marks, 0);
        }
    }

    #[test]
    fn counter_reset_saturates_instead_of_underflowing() {
        let first = row(vec![(10_000, 0, 0)], 0, 100, 50, 0);
        let (_diff, seed) = direction_diff(&first, None);
        let reset = row(vec![(5, 0, 0)], 0, 1, 0, 0);
        let (diff, _) = direction_diff(&reset, Some(&seed));
        assert_eq!(diff.tins[0].sent_bytes, 0, "reset interval carries no rate");
        assert_eq!(diff.drops, 0);
        assert_eq!(diff.marks, 0);
    }

    #[test]
    fn empty_tins_map_to_an_empty_diff() {
        let fifo = QdiscDirectionRow {
            tins: Vec::new(),
            memory_used: 0,
            queue_depth_pkts: 0,
            drop_pkts: 0,
            drop_pkts_queue_full: 0,
            aqm_marks: 0,
        };
        let (diff, seed) = direction_diff(&fifo, None);
        assert!(diff.tins.is_empty());
        assert!(seed.tins_sent_bytes.is_empty());
    }
}
