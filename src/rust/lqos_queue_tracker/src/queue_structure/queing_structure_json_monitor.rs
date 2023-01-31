use std::time::Duration;

use crate::queue_structure::{
    queue_network::QueueNetwork, queue_node::QueueNode, read_queueing_structure,
};
use lazy_static::*;
use parking_lot::RwLock;
use thiserror::Error;
use tokio::task::spawn_blocking;
use log::{info, error};

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
fn watch_for_queueing_structure_changing() -> Result<(), QueueWatcherError> {
    info!("Starting the queue structure monitor.");
    use notify::{Config, RecursiveMode, Watcher};

    // Obtain the path to watch
    let watch_path = QueueNetwork::path();
    if watch_path.is_err() {
        error!("Could not create path for queuingStructure.json");
        return Err(QueueWatcherError::CannotCreatePath);
    }
    let watch_path = watch_path.unwrap();

    // File notify doesn't work for files that don't exist
    // It's quite possible that a user is just starting, and will
    // not have a queueingStructure.json yet - so we need to keep
    // trying to obtain one.

    if !watch_path.exists() {
        info!("queueingStructure.json does not exist yet.");
        loop {
            std::thread::sleep(Duration::from_secs(30));
            if watch_path.exists() {
                info!("queueingStructure.json was just created. Sleeping 1 second and watching it.");
                std::thread::sleep(Duration::from_secs(1));
                QUEUE_STRUCTURE.write().update();
                break;
            }
        }
    }

    // Build the monitor
    let (tx, rx) = std::sync::mpsc::channel();
    let watcher = notify::RecommendedWatcher::new(tx, Config::default());
    if watcher.is_err() {
        error!("Could not create file watcher for queueingStructure.json");
        error!("{:?}", watcher);
        return Err(QueueWatcherError::WatcherFail);
    }
    let mut watcher = watcher.unwrap();

    // Start monitoring
    let result = watcher.watch(&watch_path, RecursiveMode::NonRecursive);
    if result.is_ok() {
        info!("Watching queueingStructure.csv for changes.");
        loop {
            let _ = rx.recv();
            log::info!("queuingStructure.csv changed");
            QUEUE_STRUCTURE.write().update();
        }
    } else {
        error!("Unable to start queueingStructure watcher.");
        error!("{:?}", watcher);
        Err(QueueWatcherError::WatcherFail)
    }
}

#[derive(Error, Debug)]
pub enum QueueWatcherError {
    #[error("Could not create the path buffer to find queuingStructure.json")]
    CannotCreatePath,
    #[error("Cannot watch queueingStructure.json")]
    WatcherFail,
}