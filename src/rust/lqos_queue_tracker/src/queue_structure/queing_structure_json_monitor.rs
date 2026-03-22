use crate::queue_structure::{
    queue_network::QueueNetwork, queue_node::QueueNode, read_queueing_structure,
};
use arc_swap::ArcSwap;
use lqos_utils::file_watcher::FileWatcher;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use thiserror::Error;
use tracing::{debug, error, info, warn};

/// Global queue structure (from `queueingStructure.json`)
pub static QUEUE_STRUCTURE: Lazy<ArcSwap<QueueStructure>> =
    Lazy::new(|| ArcSwap::new(Arc::new(QueueStructure::new())));
/// Global effective node-rate overlay derived from `queuingStructure.json`.
///
/// This contains only named queue nodes that map cleanly back to authored network-tree
/// entries. Circuit/device rows and generated placeholder nodes are intentionally excluded.
pub static EFFECTIVE_NODE_RATES: Lazy<ArcSwap<HashMap<String, (f64, f64)>>> = Lazy::new(|| {
    let initial = QUEUE_STRUCTURE.load();
    let rates = initial
        .maybe_queues
        .as_deref()
        .map(build_effective_node_rates)
        .unwrap_or_default();
    ArcSwap::new(Arc::new(rates))
});
/// Set to true when the queue structure changes. This is here rather than in StormGuard
/// to avoid circular dependencies.
pub static QUEUE_STRUCTURE_CHANGED_STORMGUARD: AtomicBool = AtomicBool::new(false);

#[allow(missing_docs)]
#[derive(Clone)]
/// Snapshot of the current flattened queue tree loaded from `queuingStructure.json`.
pub struct QueueStructure {
    pub maybe_queues: Option<Vec<QueueNode>>,
}

impl QueueStructure {
    fn new() -> Self {
        if let Ok(queues) = read_queueing_structure() {
            Self {
                maybe_queues: Some(queues),
            }
        } else {
            Self { maybe_queues: None }
        }
    }
}

fn build_effective_node_rates(queues: &[QueueNode]) -> HashMap<String, (f64, f64)> {
    let mut rates = HashMap::with_capacity(queues.len());
    for queue in queues {
        let Some(name) = queue.name.as_ref() else {
            continue;
        };
        if name.starts_with("Generated_PN_")
            || queue.circuit_id.is_some()
            || queue.device_id.is_some()
        {
            continue;
        }

        rates.insert(
            name.clone(),
            (
                queue.download_bandwidth_mbps as f64,
                queue.upload_bandwidth_mbps as f64,
            ),
        );
    }
    rates
}

/// Global file watched for `queueStructure.json`.
/// Reloads the queue structure when it is available.
pub fn spawn_queue_structure_monitor() -> anyhow::Result<()> {
    std::thread::Builder::new()
        .name("Queue Structure Monitor".to_string())
        .spawn(|| {
            if let Err(e) = watch_for_queueing_structure_changing() {
                error!("Error watching for queueingStructure.json: {:?}", e);
            }
        })?;

    Ok(())
}

fn update_queue_structure() {
    debug!("queueingStructure.json reload requested");
    match read_queueing_structure() {
        Ok(queues) => {
            let new_queue_structure = QueueStructure {
                maybe_queues: Some(queues.clone()),
            };
            let effective_node_rates = build_effective_node_rates(&queues);
            QUEUE_STRUCTURE.store(Arc::new(new_queue_structure));
            EFFECTIVE_NODE_RATES.store(Arc::new(effective_node_rates));
            QUEUE_STRUCTURE_CHANGED_STORMGUARD.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Err(err) => {
            if QUEUE_STRUCTURE.load().maybe_queues.is_some() {
                warn!(
                    "Failed to reload queuingStructure.json ({err:?}); preserving last-known-good snapshot"
                );
            } else {
                warn!(
                    "Failed to load queuingStructure.json ({err:?}); leaving queue structure unavailable"
                );
                QUEUE_STRUCTURE.store(Arc::new(QueueStructure { maybe_queues: None }));
                EFFECTIVE_NODE_RATES.store(Arc::new(HashMap::new()));
            }
        }
    }
}

/// Fires up a Linux file system watcher than notifies
/// when `queuingStructure.json` changes, and triggers a reload.
fn watch_for_queueing_structure_changing() -> Result<(), QueueWatcherError> {
    // Get the path to watch
    let Ok(watch_path) = QueueNetwork::path() else {
        error!("Could not create path for queuingStructure.json");
        return Err(QueueWatcherError::CannotCreatePath);
    };

    // Do the watching
    let mut watcher = FileWatcher::new("queueingStructure.json", watch_path);
    watcher.set_file_created_callback(update_queue_structure);
    watcher.set_file_changed_callback(update_queue_structure);
    loop {
        let retval = watcher.watch();
        if retval.is_err() {
            info!("File watcher returned {retval:?}");
        }
    }
}

#[derive(Error, Debug)]
pub enum QueueWatcherError {
    #[error("Could not create the path buffer to find queuingStructure.json")]
    CannotCreatePath,
}
