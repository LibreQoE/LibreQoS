use crate::queue_structure::{
  queue_network::QueueNetwork, queue_node::QueueNode, read_queueing_structure,
};
use lazy_static::*;
use log::{error, info};
use lqos_utils::file_watcher::FileWatcher;
use parking_lot::RwLock;
use thiserror::Error;
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
      Self { maybe_queues: Some(queues) }
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

fn update_queue_structure() {
  info!("queueingStructure.json reloaded");
  QUEUE_STRUCTURE.write().update();
}

/// Fires up a Linux file system watcher than notifies
/// when `ShapedDevices.csv` changes, and triggers a reload.
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
