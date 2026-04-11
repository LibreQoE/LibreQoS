use anyhow::Result;
use lqos_config::NetworkJson;
use lqos_utils::file_watcher::FileWatcher;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::path::PathBuf;
use tracing::{debug, error, info, warn};

pub static NETWORK_JSON: Lazy<RwLock<NetworkJson>> =
    Lazy::new(|| RwLock::new(NetworkJson::default()));

pub fn network_json_watcher() -> Result<()> {
    std::thread::Builder::new()
        .name("Active Network Tree Watcher".to_string())
        .spawn(|| {
            debug!("Watching for active network tree changes");
            if let Err(e) = watch_for_network_json_changing() {
                error!("Error watching for active network tree changes: {:?}", e);
            }
        })?;
    Ok(())
}

fn active_network_tree_path() -> Result<PathBuf> {
    NetworkJson::path()
        .map_err(|_| anyhow::Error::msg("Unable to create path for active network tree"))
}

/// Fires up a Linux file system watcher that notifies
/// when the active runtime tree changes and triggers a reload.
fn watch_for_network_json_changing() -> Result<()> {
    let watch_path = active_network_tree_path()?;
    let watch_name = watch_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("active network tree")
        .to_string();

    let mut watcher = FileWatcher::new(&watch_name, watch_path);
    watcher.set_file_exists_callback(load_network_json);
    watcher.set_file_created_callback(load_network_json);
    watcher.set_file_changed_callback(load_network_json);
    loop {
        let result = watcher.watch();
        info!("active network tree watcher returned: {result:?}");
    }
}

fn load_network_json() {
    let njs = NetworkJson::load();
    if let Ok(njs) = njs {
        let mut nj = NETWORK_JSON.write();
        *nj = njs;
        super::invalidate_circuit_live_snapshot();
        super::invalidate_executive_cache_snapshot();
        crate::throughput_tracker::THROUGHPUT_TRACKER.refresh_circuit_ids(&nj);
    } else {
        warn!("Unable to load active runtime network tree");
    }
}
