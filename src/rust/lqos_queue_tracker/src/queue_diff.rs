use crate::queue_types::QueueType;
use serde::Serialize;
use thiserror::Error;
use tracing::{debug, warn};

#[derive(Debug, Clone, Serialize)]
pub enum QueueDiff {
    None,
    //    Mq,
    //    Htb,
    FqCodel(FqCodelDiff),
    Cake(CakeDiff),
    //    ClsAct,
}

pub(crate) fn make_queue_diff(
    previous: &QueueType,
    current: &QueueType,
) -> Result<QueueDiff, QueueDiffError> {
    match previous {
        QueueType::FqCodel(..) => match current {
            QueueType::FqCodel(..) => Ok(fq_codel_diff(previous, current)?),
            _ => {
                warn!(
                    previous = queue_kind(previous),
                    current = queue_kind(current),
                    "Queue type changed; resetting queue diff sample"
                );
                Ok(QueueDiff::None)
            }
        },
        QueueType::Cake(..) => match current {
            QueueType::Cake(..) => Ok(cake_diff(previous, current)?),
            _ => {
                warn!(
                    previous = queue_kind(previous),
                    current = queue_kind(current),
                    "Queue type changed; resetting queue diff sample"
                );
                Ok(QueueDiff::None)
            }
        },
        _ => {
            warn!(
                previous = queue_kind(previous),
                current = queue_kind(current),
                "Queue diff unavailable for queue type; resetting queue diff sample"
            );
            Ok(QueueDiff::None)
        }
    }
}

fn queue_kind(queue: &QueueType) -> &'static str {
    match queue {
        QueueType::FqCodel(_) => "fq_codel",
        QueueType::Cake(_) => "cake",
        QueueType::Mq(_) => "mq",
        QueueType::Htb(_) => "htb",
        QueueType::ClsAct => "clsact",
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct CakeDiff {
    pub bytes: u64,
    pub packets: u64,
    pub qlen: u64,
    pub tins: Vec<CakeDiffTin>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CakeDiffTin {
    pub sent_bytes: u64,
    pub backlog_bytes: u64,
    pub drops: u64,
    pub marks: u64,
    pub base_delay_us: u64,
    pub sent_packets: u64,
    pub peak_delay_us: u64,
    pub avg_delay_us: u64,
    pub way_indirect_hits: u64,
    pub way_misses: u64,
    pub way_collisions: u64,
    pub sparse_flows: u64,
    pub bulk_flows: u64,
    pub unresponsive_flows: u64,
}

fn cake_diff(previous: &QueueType, current: &QueueType) -> Result<QueueDiff, QueueDiffError> {
    // TODO: Wrapping Handler
    if let QueueType::Cake(prev) = previous
        && let QueueType::Cake(new) = current
    {
        if prev.tins.len() != new.tins.len() {
            debug!(
                previous_tins = prev.tins.len(),
                current_tins = new.tins.len(),
                "CAKE tin count changed; resetting queue diff sample"
            );
            return Ok(QueueDiff::None);
        }

        let tins = new
            .tins
            .iter()
            .zip(prev.tins.iter())
            .map(|(new, prev)| CakeDiffTin {
                sent_bytes: new.sent_bytes.saturating_sub(prev.sent_bytes),
                backlog_bytes: new.backlog_bytes,
                drops: new.drops.saturating_sub(prev.drops),
                marks: new.ecn_marks.saturating_sub(prev.ecn_marks),
                base_delay_us: new.base_delay_us,
                sent_packets: new.sent_packets.saturating_sub(prev.sent_packets),
                peak_delay_us: new.peak_delay_us,
                avg_delay_us: new.avg_delay_us,
                way_indirect_hits: new.way_indirect_hits.saturating_sub(prev.way_indirect_hits),
                way_misses: new.way_misses.saturating_sub(prev.way_misses),
                way_collisions: new.way_collisions.saturating_sub(prev.way_collisions),
                sparse_flows: new.sparse_flows,
                bulk_flows: new.bulk_flows,
                unresponsive_flows: new.unresponsive_flows,
            })
            .collect();
        return Ok(QueueDiff::Cake(CakeDiff {
            bytes: new.bytes.saturating_sub(prev.bytes),
            packets: new.packets.saturating_sub(prev.packets),
            qlen: new.qlen,
            tins,
        }));
    }
    Err(QueueDiffError::NotImplemented)
}

#[derive(Debug, Error)]
pub enum QueueDiffError {
    #[error("Not implemented")]
    NotImplemented,
}
#[derive(Serialize, Clone, Debug)]
pub struct FqCodelDiff {
    pub bytes: u64,
    pub packets: u64,
    pub backlog: u64,
    pub flows: u64,
    pub ddrops: u64,
}

fn fq_codel_diff(previous: &QueueType, current: &QueueType) -> Result<QueueDiff, QueueDiffError> {
    if let QueueType::FqCodel(prev) = previous
        && let QueueType::FqCodel(new) = current
    {
        // Delta counters; backlog and flows are instantaneous
        let diff = FqCodelDiff {
            bytes: new.bytes.saturating_sub(prev.bytes),
            packets: new.packets.saturating_sub(prev.packets),
            backlog: new.backlog,
            flows: new.options.flows,
            ddrops: new.drop_overlimit.saturating_sub(prev.drop_overlimit),
        };
        return Ok(QueueDiff::FqCodel(diff));
    }
    Err(QueueDiffError::NotImplemented)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deserialize_tc_tree;

    #[test]
    fn queue_type_change_resets_diff_sample() {
        let fq_codel = deserialize_tc_tree(
            r#"[{
                "kind":"fq_codel",
                "handle":"9000:",
                "parent":"1:2",
                "options":{},
                "bytes":1000,
                "packets":10
            }]"#,
        )
        .expect("fq_codel qdisc should parse")
        .remove(0);
        let cake = deserialize_tc_tree(
            r#"[{
                "kind":"cake",
                "handle":"9001:",
                "parent":"1:2",
                "options":{},
                "bytes":2000,
                "packets":20,
                "tins":[]
            }]"#,
        )
        .expect("cake qdisc should parse")
        .remove(0);

        let diff = make_queue_diff(&fq_codel, &cake).expect("type changes should not error");

        assert!(matches!(diff, QueueDiff::None));
    }

    #[test]
    fn cake_tin_count_change_resets_diff_sample() {
        let previous = deserialize_tc_tree(
            r#"[{
                "kind":"cake",
                "handle":"9001:",
                "parent":"1:2",
                "options":{},
                "bytes":1000,
                "packets":10,
                "tins":[{
                    "sent_bytes":100,
                    "backlog_bytes":3,
                    "base_delay_us":1
                }]
            }]"#,
        )
        .expect("previous cake qdisc should parse")
        .remove(0);
        let current = deserialize_tc_tree(
            r#"[{
                "kind":"cake",
                "handle":"9001:",
                "parent":"1:2",
                "options":{},
                "bytes":2000,
                "packets":20,
                "tins":[]
            }]"#,
        )
        .expect("current cake qdisc should parse")
        .remove(0);

        let diff = make_queue_diff(&previous, &current)
            .expect("tin count changes should reset without error");

        assert!(matches!(diff, QueueDiff::None));
    }

    #[test]
    fn cake_diff_includes_extended_delta_and_gauge_fields() {
        let previous = deserialize_tc_tree(
            r#"[{
                "kind":"cake",
                "handle":"9001:",
                "parent":"1:2",
                "options":{},
                "bytes":1000,
                "packets":10,
                "qlen":1,
                "tins":[{
                    "sent_bytes":100,
                    "backlog_bytes":3,
                    "peak_delay_us":10,
                    "avg_delay_us":5,
                    "base_delay_us":1,
                    "sent_packets":5000000000,
                    "way_indirect_hits":70000,
                    "way_misses":80000,
                    "way_collisions":90000,
                    "drops":7,
                    "ecn_mark":9,
                    "sparse_flows":1,
                    "bulk_flows":2,
                    "unresponsive_flows":3
                }]
            }]"#,
        )
        .expect("previous cake qdisc should parse")
        .remove(0);
        let current = deserialize_tc_tree(
            r#"[{
                "kind":"cake",
                "handle":"9001:",
                "parent":"1:2",
                "options":{},
                "bytes":1600,
                "packets":18,
                "qlen":2,
                "tins":[{
                    "sent_bytes":175,
                    "backlog_bytes":4,
                    "peak_delay_us":25,
                    "avg_delay_us":8,
                    "base_delay_us":2,
                    "sent_packets":5000000010,
                    "way_indirect_hits":70011,
                    "way_misses":80010,
                    "way_collisions":90003,
                    "drops":8,
                    "ecn_mark":13,
                    "sparse_flows":4,
                    "bulk_flows":5,
                    "unresponsive_flows":6
                }]
            }]"#,
        )
        .expect("current cake qdisc should parse")
        .remove(0);

        let diff = make_queue_diff(&previous, &current).expect("cake diff should be available");
        let QueueDiff::Cake(diff) = diff else {
            panic!("expected cake diff");
        };
        let tin = diff.tins.first().expect("cake diff should include one tin");

        assert_eq!(tin.sent_bytes, 75);
        assert_eq!(tin.drops, 1);
        assert_eq!(tin.marks, 4);
        assert_eq!(tin.sent_packets, 10);
        assert_eq!(tin.way_indirect_hits, 11);
        assert_eq!(tin.way_misses, 10);
        assert_eq!(tin.way_collisions, 3);
        assert_eq!(tin.backlog_bytes, 4);
        assert_eq!(tin.peak_delay_us, 25);
        assert_eq!(tin.avg_delay_us, 8);
        assert_eq!(tin.base_delay_us, 2);
        assert_eq!(tin.sparse_flows, 4);
        assert_eq!(tin.bulk_flows, 5);
        assert_eq!(tin.unresponsive_flows, 6);
    }

    #[test]
    fn cake_diff_saturates_extended_counter_resets() {
        let previous = deserialize_tc_tree(
            r#"[{
                "kind":"cake",
                "handle":"9001:",
                "parent":"1:2",
                "options":{},
                "bytes":5000,
                "packets":100,
                "qlen":1,
                "tins":[{
                    "sent_bytes":5000,
                    "backlog_bytes":3,
                    "peak_delay_us":10,
                    "avg_delay_us":5,
                    "base_delay_us":1,
                    "sent_packets":5000000000,
                    "way_indirect_hits":70000,
                    "way_misses":80000,
                    "way_collisions":90000,
                    "drops":7,
                    "ecn_mark":9,
                    "sparse_flows":1,
                    "bulk_flows":2,
                    "unresponsive_flows":3
                }]
            }]"#,
        )
        .expect("previous cake qdisc should parse")
        .remove(0);
        let current = deserialize_tc_tree(
            r#"[{
                "kind":"cake",
                "handle":"9001:",
                "parent":"1:2",
                "options":{},
                "bytes":1000,
                "packets":10,
                "qlen":2,
                "tins":[{
                    "sent_bytes":1000,
                    "backlog_bytes":4,
                    "peak_delay_us":25,
                    "avg_delay_us":8,
                    "base_delay_us":2,
                    "sent_packets":100,
                    "way_indirect_hits":10,
                    "way_misses":20,
                    "way_collisions":30,
                    "drops":1,
                    "ecn_mark":2,
                    "sparse_flows":4,
                    "bulk_flows":5,
                    "unresponsive_flows":6
                }]
            }]"#,
        )
        .expect("current cake qdisc should parse")
        .remove(0);

        let diff = make_queue_diff(&previous, &current).expect("cake diff should be available");
        let QueueDiff::Cake(diff) = diff else {
            panic!("expected cake diff");
        };
        let tin = diff.tins.first().expect("cake diff should include one tin");

        assert_eq!(tin.sent_packets, 0);
        assert_eq!(tin.way_indirect_hits, 0);
        assert_eq!(tin.way_misses, 0);
        assert_eq!(tin.way_collisions, 0);
        assert_eq!(tin.peak_delay_us, 25);
        assert_eq!(tin.avg_delay_us, 8);
        assert_eq!(tin.sparse_flows, 4);
        assert_eq!(tin.bulk_flows, 5);
        assert_eq!(tin.unresponsive_flows, 6);
    }
}
