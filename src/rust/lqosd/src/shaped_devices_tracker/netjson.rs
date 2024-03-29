use anyhow::Result;
use log::{error, info, warn};
use lqos_config::NetworkJson;
use lqos_utils::file_watcher::FileWatcher;
use once_cell::sync::Lazy;
use std::sync::RwLock;
use tokio::task::spawn_blocking;

pub static NETWORK_JSON: Lazy<RwLock<NetworkJson>> =
  Lazy::new(|| RwLock::new(NetworkJson::default()));

pub async fn network_json_watcher() {
  spawn_blocking(|| {
    info!("Watching for network.kson changes");
    let _ = watch_for_network_json_changing();
  });
}

/// Fires up a Linux file system watcher than notifies
/// when `network.json` changes, and triggers a reload.
fn watch_for_network_json_changing() -> Result<()> {
  let watch_path = NetworkJson::path();
  if watch_path.is_err() {
    error!("Unable to generate path for network.json");
    return Err(anyhow::Error::msg("Unable to create path for network.json"));
  }
  let watch_path = watch_path.unwrap();

  let mut watcher = FileWatcher::new("network.json", watch_path);
  watcher.set_file_exists_callback(load_network_json);
  watcher.set_file_created_callback(load_network_json);
  watcher.set_file_changed_callback(load_network_json);
  loop {
    let result = watcher.watch();
    info!("network.json watcher returned: {result:?}");
  }
}

fn load_network_json() {
  let njs = NetworkJson::load();
  if let Ok(njs) = njs {
    let mut write_lock = NETWORK_JSON.write().unwrap();
    *write_lock = njs;
    std::mem::drop(write_lock);
    crate::throughput_tracker::THROUGHPUT_TRACKER
      .refresh_circuit_ids();
  } else {
    warn!("Unable to load network.json");
  }
}
