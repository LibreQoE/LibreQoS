use crate::queue_types::QueueType;
use serde::Serialize;
use thiserror::Error;
use tracing::error;

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
                error!(
                    "Queue diffs are not implemented for FqCodel to {:?}",
                    current
                );
                Err(QueueDiffError::NotImplemented)
            }
        },
        QueueType::Cake(..) => match current {
            QueueType::Cake(..) => Ok(cake_diff(previous, current)?),
            _ => {
                error!("Queue diffs are not implemented for Cake to {:?}", current);
                Err(QueueDiffError::NotImplemented)
            }
        },
        _ => {
            error!("Queue diffs are not implemented for {:?}", current);
            Err(QueueDiffError::NotImplemented)
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct CakeDiff {
    pub bytes: u64,
    pub packets: u32,
    pub qlen: u32,
    pub tins: Vec<CakeDiffTin>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CakeDiffTin {
    pub sent_bytes: u64,
    pub backlog_bytes: u32,
    pub drops: u32,
    pub marks: u32,
    pub base_delay_us: u32,
}

fn cake_diff(previous: &QueueType, current: &QueueType) -> Result<QueueDiff, QueueDiffError> {
    // TODO: Wrapping Handler
    if let QueueType::Cake(prev) = previous {
        if let QueueType::Cake(new) = current {
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
    pub packets: u32,
    pub backlog: u32,
    pub flows: u16,
    pub ddrops: u32,
}

fn fq_codel_diff(previous: &QueueType, current: &QueueType) -> Result<QueueDiff, QueueDiffError> {
    if let QueueType::FqCodel(prev) = previous {
        if let QueueType::FqCodel(new) = current {
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
    }
    Err(QueueDiffError::NotImplemented)
}
