use crate::config::WatchingSiteDependency;
use anyhow::{Result, bail};
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

pub fn find_queue_dependents(parent_name: &str) -> Result<Vec<WatchingSiteDependency>> {
    let Some(queues) = &QUEUE_STRUCTURE.load().maybe_queues else {
        bail!("No queue structure - cannot start");
    };

    let Some(queue) = queues.iter().find(|n| {
        if let Some(q) = &n.name {
            *q == parent_name
        } else {
            false
        }
    }) else {
        bail!("Queue {} not found in queue structure", parent_name);
    };

    let mut dependents = Vec::new();
    for q in queues.iter() {
        if queue.class_id == q.parent_class_id && q.parent_node.is_none() {
            // If they don't have any CAKE descendents, they are good.
            if !queues
                .iter()
                .any(|q| q.parent_class_id == q.class_id && q.parent_node.is_some())
            {
                dependents.push(WatchingSiteDependency {
                    name: q.clone().name.unwrap_or_default(),
                    class_id: q.class_id,
                    original_max_download_mbps: q.download_bandwidth_mbps,
                    original_max_upload_mbps: q.upload_bandwidth_mbps,
                });
            }
        }
    }

    Ok(dependents)
}
