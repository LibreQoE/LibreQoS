use anyhow::{bail, Result};
use lqos_queue_tracker::QUEUE_STRUCTURE;

pub fn find_queue_bandwidth(name: &str) -> Result<(u64, u64)> {
    let Some(queues) = &QUEUE_STRUCTURE.load().maybe_queues else {
        bail!("No queue structure - cannot start");
    };
    
    let Some(queue) = queues.iter().find(|n| {
        if let Some(q) = &n.name {
            *q == name
        } else {
            false
        }
    }) else {
        bail!("Queue {} not found in queue structure", name);
    };
    
    Ok((queue.download_bandwidth_mbps, queue.upload_bandwidth_mbps))
}