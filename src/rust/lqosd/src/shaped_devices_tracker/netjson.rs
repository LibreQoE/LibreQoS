use anyhow::Result;
use lqos_config::NetworkJson;
use lqos_utils::file_watcher::FileWatcher;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use tracing::{debug, error, info, warn};

pub static NETWORK_JSON: Lazy<RwLock<NetworkJson>> =
    Lazy::new(|| RwLock::new(NetworkJson::default()));

pub fn network_json_watcher() -> Result<()> {
    std::thread::Builder::new()
        .name("NetJson Watcher".to_string())
        .spawn(|| {
            debug!("Watching for network.kson changes");
            if let Err(e) = watch_for_network_json_changing() {
                error!("Error watching for network.json changes: {:?}", e);
            }
        })?;
    Ok(())
}

/// Fires up a Linux file system watcher that notifies
/// when `network.json` changes and triggers a reload.
fn watch_for_network_json_changing() -> Result<()> {
    let watch_path = NetworkJson::path();
    let Ok(watch_path) = watch_path else {
        error!("Unable to generate path for network.json");
        return Err(anyhow::Error::msg("Unable to create path for network.json"));
    };

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
        let mut nj = NETWORK_JSON.write();
        *nj = njs;
        crate::throughput_tracker::THROUGHPUT_TRACKER.refresh_circuit_ids(&nj);
    } else {
        warn!("Unable to load network.json");
    }
}
