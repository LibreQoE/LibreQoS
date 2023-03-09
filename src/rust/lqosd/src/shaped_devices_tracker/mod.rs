use anyhow::Result;
use log::{error, info, warn};
use lqos_bus::BusResponse;
use lqos_config::ConfigShapedDevices;
use lqos_utils::file_watcher::FileWatcher;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use tokio::task::spawn_blocking;
mod netjson;
pub use netjson::*;

pub static SHAPED_DEVICES: Lazy<RwLock<ConfigShapedDevices>> =
  Lazy::new(|| RwLock::new(ConfigShapedDevices::default()));

fn load_shaped_devices() {
  info!("ShapedDevices.csv has changed. Attempting to load it.");
  let shaped_devices = ConfigShapedDevices::load();
  if let Ok(new_file) = shaped_devices {
    info!("ShapedDevices.csv loaded");
    *SHAPED_DEVICES.write() = new_file;
    crate::throughput_tracker::THROUGHPUT_TRACKER.write().refresh_circuit_ids();
  } else {
    warn!("ShapedDevices.csv failed to load, see previous error messages. Reverting to empty set.");
    *SHAPED_DEVICES.write() = ConfigShapedDevices::default();
  }
}

pub async fn shaped_devices_watcher() {
  spawn_blocking(|| {
    info!("Watching for ShapedDevices.csv changes");
    let _ = watch_for_shaped_devices_changing();
  });
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

pub fn get_one_network_map_layer(parent_idx: usize) -> BusResponse {
  let net_json = NETWORK_JSON.read();
  if let Some(parent) = net_json.get_cloned_entry_by_index(parent_idx) {
    let mut nodes = vec![(parent_idx, parent)];
    nodes.extend_from_slice(&net_json.get_cloned_children(parent_idx));
    BusResponse::NetworkMap(nodes)
  } else {
    BusResponse::Fail("No such node".to_string())
  }
}