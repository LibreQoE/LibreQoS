use crate::queue_structure::{
    queue_network::QueueNetwork, queue_node::QueueNode, read_queueing_structure,
};
use anyhow::Result;
use lazy_static::*;
use parking_lot::RwLock;
use tokio::task::spawn_blocking;

lazy_static! {
    /// Global storage of the shaped devices csv data.
    /// Updated by the file system watcher whenever
    /// the underlying file changes.
    pub(crate) static ref QUEUE_STRUCTURE : RwLock<QueueStructure> = RwLock::new(QueueStructure::new());
}

#[derive(Clone)]
pub(crate) struct QueueStructure {
    pub(crate) maybe_queues: Option<Vec<QueueNode>>,
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

    fn update(&mut self) {
        if let Ok(queues) = read_queueing_structure() {
            self.maybe_queues = Some(queues);
        } else {
            self.maybe_queues = None;
        }
    }
}

/// Global file watched for `queueStructure.json`.
/// Reloads the queue structure when it is available.
pub async fn spawn_queue_structure_monitor() {
    spawn_blocking(|| {
        let _ = watch_for_queueing_structure_changing();
    });
}

/// Fires up a Linux file system watcher than notifies
/// when `ShapedDevices.csv` changes, and triggers a reload.
fn watch_for_queueing_structure_changing() -> Result<()> {
    use notify::{Config, RecursiveMode, Watcher};

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::RecommendedWatcher::new(tx, Config::default())?;

    watcher.watch(&QueueNetwork::path()?, RecursiveMode::NonRecursive)?;
    loop {
        let _ = rx.recv();
        log::info!("queuingStructure.csv changed");
        QUEUE_STRUCTURE.write().update();
    }
}
