use serde::Serialize;
use anyhow::Result;
use super::QueueType;

#[derive(Debug, Clone, Serialize)]
pub enum QueueDiff {
    None,
//    Mq,
//    Htb,
//    FqCodel,
    Cake(CakeDiff),
//    ClsAct,
}

pub(crate) fn make_queue_diff(previous: &QueueType, current: &QueueType) -> Result<QueueDiff> {
    match previous {
        QueueType::Cake(..) => {
            match current {
                QueueType::Cake(..) => Ok(cake_diff(previous, current)?),
                _ => Err(anyhow::Error::msg("Not implemented"))
            }
        }
        _ => Err(anyhow::Error::msg("Not implemented"))
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
    pub avg_delay_us: u64,
}

fn cake_diff(previous: &QueueType, current: &QueueType) -> Result<QueueDiff> {
    // TODO: Wrapping Handler
    if let QueueType::Cake(prev) = previous {
        if let QueueType::Cake(new) = current {
            let tins = new.tins.iter().zip(prev.tins.iter()).map(|(new, prev)| {
                //println!("{} - {} = {}", new.sent_bytes, prev.sent_bytes, new.sent_bytes -prev.sent_bytes);
                CakeDiffTin {
                    sent_bytes: new.sent_bytes - prev.sent_bytes,
                    backlog_bytes: new.backlog_bytes,
                    drops: new.drops - prev.drops,
                    marks: new.ecn_marks - prev.ecn_marks,
                    avg_delay_us: new.avg_delay_us,
                }
            }).collect();
            return Ok(QueueDiff::Cake(CakeDiff{
                bytes: new.bytes - prev.bytes,
                packets: new.packets - prev.packets,
                qlen: new.qlen,
                tins,
            }));
        }
    }
    Err(anyhow::Error::msg("Not implemented"))
}