use crate::queue_types::QueueType;
use serde::Serialize;
use thiserror::Error;
use tracing::warn;

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
}

fn cake_diff(previous: &QueueType, current: &QueueType) -> Result<QueueDiff, QueueDiffError> {
    // TODO: Wrapping Handler
    if let QueueType::Cake(prev) = previous
        && let QueueType::Cake(new) = current
    {
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
}
