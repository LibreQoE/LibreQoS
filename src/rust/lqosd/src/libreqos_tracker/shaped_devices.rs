//! Tracks changes to the ShapedDevices.csv file in LibreQoS.

use lazy_static::*;
use lqos_config::ConfigShapedDevices;
use parking_lot::RwLock;
use anyhow::Result;
use tokio::task::spawn_blocking;

lazy_static! {
    /// Global storage of the shaped devices csv data.
    /// Updated by the file system watcher whenever
    /// the underlying file changes.
    pub(crate) static ref SHAPED_DEVICES : RwLock<ConfigShapedDevices> = RwLock::new(ConfigShapedDevices::load().unwrap());
}

pub async fn spawn_shaped_devices_monitor() {
    spawn_blocking(|| {
        let _ = watch_for_shaped_devices_changing();
    });
}

/// Fires up a Linux file system watcher than notifies
/// when `ShapedDevices.csv` changes, and triggers a reload.
fn watch_for_shaped_devices_changing() -> Result<()> {
    use notify::{Watcher, RecursiveMode, Config};

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::RecommendedWatcher::new(tx, Config::default())?;

    watcher.watch(&ConfigShapedDevices::path()?, RecursiveMode::NonRecursive)?;
    loop {
        let _ = rx.recv();
        if let Ok(new_file) = ConfigShapedDevices::load() {
            println!("ShapedDevices.csv changed");
            *SHAPED_DEVICES.write() = new_file;
        }
    }
}