//! Tracks changes to the ShapedDevices.csv file in LibreQoS.

use crate::libreqos_tracker::queueing_structure::{
    read_queueing_structure, QueueNetwork, QueueNode,
};
use anyhow::Result;
use lazy_static::*;
use parking_lot::RwLock;
use tokio::task::spawn_blocking;

lazy_static! {
    /// Global storage of the shaped devices csv data.
    /// Updated by the file system watcher whenever
    /// the underlying file changes.
    pub(crate) static ref QUEUE_STRUCTURE : RwLock<Result<Vec<QueueNode>>> = RwLock::new(read_queueing_structure());
}

pub async fn spawn_queue_structure_monitor() {
    spawn_blocking(|| {
        let _ = watch_for_shaped_devices_changing();
    });
}

/// Fires up a Linux file system watcher than notifies
/// when `ShapedDevices.csv` changes, and triggers a reload.
fn watch_for_shaped_devices_changing() -> Result<()> {
    use notify::{Config, RecursiveMode, Watcher};

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::RecommendedWatcher::new(tx, Config::default())?;

    watcher.watch(&QueueNetwork::path()?, RecursiveMode::NonRecursive)?;
    loop {
        let _ = rx.recv();
        let new_file = read_queueing_structure();
        log::info!("queuingStructure.csv changed");
        *QUEUE_STRUCTURE.write() = new_file;
    }
}
