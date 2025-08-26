use crate::queue_structure::{
    queue_network::QueueNetwork, queue_node::QueueNode, read_queueing_structure,
};
use crate::tracking::ALL_QUEUE_SUMMARY;
use arc_swap::ArcSwap;
use lqos_utils::file_watcher::FileWatcher;
use once_cell::sync::Lazy;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, error, info};

/// Global queue structure (from `queueingStructure.json`)
pub static QUEUE_STRUCTURE: Lazy<ArcSwap<QueueStructure>> = Lazy::new(|| ArcSwap::new(Arc::new(QueueStructure::new())));
/// Set to true when the queue structure changes. This is here rather than in StormGuard
/// to avoid circular dependencies.
pub static QUEUE_STRUCTURE_CHANGED_STORMGUARD: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
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
    debug!("queueingStructure.json reloaded");
    let new_queue_structure = QueueStructure::new();
    ALL_QUEUE_SUMMARY.clear();
    QUEUE_STRUCTURE.store(Arc::new(new_queue_structure));
    QUEUE_STRUCTURE_CHANGED_STORMGUARD.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Fires up a Linux file system watcher than notifies
/// when `queuingStructure.json` changes, and triggers a reload.
fn watch_for_queueing_structure_changing() -> Result<(), QueueWatcherError> {
    // Obtain the path to watch
    let watch_path = QueueNetwork::path();
    if watch_path.is_err() {
        error!("Could not create path for queuingStructure.json");
        return Err(QueueWatcherError::CannotCreatePath);
    }
    let watch_path = watch_path.unwrap();

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
