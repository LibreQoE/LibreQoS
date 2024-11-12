use std::net::IpAddr;
use anyhow::Result;
use tracing::{debug, error, info, warn};
use lqos_bus::{BusResponse, Circuit};
use lqos_config::{ConfigShapedDevices, NetworkJsonTransport};
use lqos_utils::file_watcher::FileWatcher;
use once_cell::sync::Lazy;
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;
use arc_swap::ArcSwap;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::time_since_boot;

mod netjson;
pub use netjson::*;

pub static SHAPED_DEVICES: Lazy<ArcSwap<ConfigShapedDevices>> =
    Lazy::new(|| ArcSwap::new(Arc::new(ConfigShapedDevices::default())));

fn load_shaped_devices() {
    debug!("ShapedDevices.csv has changed. Attempting to load it.");
    let shaped_devices = ConfigShapedDevices::load();
    if let Ok(new_file) = shaped_devices {
        debug!("ShapedDevices.csv loaded");
        SHAPED_DEVICES.store(Arc::new(new_file));
        let nj = NETWORK_JSON.read().unwrap();
    } else {
        warn!("ShapedDevices.csv failed to load, see previous error messages. Reverting to empty set.");
        SHAPED_DEVICES.store(Arc::new(ConfigShapedDevices::default()));
    }
}

pub fn shaped_devices_watcher() -> Result<()> {
    std::thread::Builder::new()
        .name("ShapedDevices Watcher".to_string())
    .spawn(|| {
        debug!("Watching for ShapedDevices.csv changes");
        if let Err(e) = watch_for_shaped_devices_changing() {
            error!("Error watching for ShapedDevices.csv: {:?}", e);
        }
    })?;
    Ok(())
}

/// Fires up a Linux file system watcher than notifies
/// when `ShapedDevices.csv` changes, and triggers a reload.
fn watch_for_shaped_devices_changing() -> Result<()> {
    let watch_path = ConfigShapedDevices::path();
    if watch_path.is_err() {
        error!("Unable to generate path for ShapedDevices.csv");
        return Err(anyhow::Error::msg(
            "Unable to create path for ShapedDevices.csv",
        ));
    }
    let watch_path = watch_path.unwrap();

    let mut watcher = FileWatcher::new("ShapedDevices.csv", watch_path);
    watcher.set_file_exists_callback(load_shaped_devices);
    watcher.set_file_created_callback(load_shaped_devices);
    watcher.set_file_changed_callback(load_shaped_devices);
    loop {
        let result = watcher.watch();
        info!("ShapedDevices watcher returned: {result:?}");
    }
}

