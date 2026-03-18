use crate::config::WatchingSiteDependency;
use anyhow::{Result, bail};
use lqos_queue_tracker::QUEUE_STRUCTURE;
use std::collections::HashSet;

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
    for candidate in queues.iter() {
        if queue.class_id == candidate.parent_class_id && candidate.parent_node.is_none() {
            // If they don't have any CAKE descendents, they are good.
            if !queues.iter().any(|child| {
                child.parent_class_id == candidate.class_id && child.parent_node.is_some()
            }) {
                dependents.push(WatchingSiteDependency {
                    name: candidate.clone().name.unwrap_or_default(),
                    class_id: candidate.class_id,
                    original_max_download_mbps: candidate.download_bandwidth_mbps,
                    original_max_upload_mbps: candidate.upload_bandwidth_mbps,
                });
            }
        }
    }

    Ok(dependents)
}

pub fn all_candidate_site_names() -> Vec<String> {
    let Some(queues) = &QUEUE_STRUCTURE.load().maybe_queues else {
        return Vec::new();
    };

    let parent_ids: HashSet<_> = queues
        .iter()
        .filter(|q| q.circuit_id.is_none())
        .map(|q| q.parent_class_id)
        .collect();

    let mut names: Vec<String> = queues
        .iter()
        .filter(|q| q.circuit_id.is_none())
        .filter_map(|q| {
            let name = q.name.as_ref()?;
            if parent_ids.contains(&q.class_id) {
                None
            } else {
                Some(name.clone())
            }
        })
        .collect();
    names.sort();
    names.dedup();
    names
}
