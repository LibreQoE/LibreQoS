use crate::{request_reload_network_json, request_reload_shaped_devices};
use anyhow::Result;
use lqos_utils::directory_watcher::{DirectoryWatcher, WatchedDirectoryError};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, warn};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum DirectoryReloadEvent {
    ShapedDevices,
    NetworkJson,
}

pub(crate) fn start_network_devices_directory_watch() -> Result<()> {
    let config = lqos_config::load_config()?;
    spawn_directory_watch_thread(
        "Network Devices Config Dir Watcher",
        "LibreQoS config directory",
        PathBuf::from(&config.lqos_directory),
    )?;
    spawn_directory_watch_thread(
        "Network Devices Topology Dir Watcher",
        "LibreQoS topology state directory",
        config.resolved_state_directory().join("topology"),
    )?;
    Ok(())
}

fn spawn_directory_watch_thread(thread_name: &str, watch_label: &str, path: PathBuf) -> Result<()> {
    let thread_name = thread_name.to_string();
    let watch_label = watch_label.to_string();
    fs::create_dir_all(&path)?;
    std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || watch_directory_loop(&watch_label, path))?;
    Ok(())
}

fn watch_directory_loop(watch_label: &str, path: PathBuf) {
    let mut watcher = DirectoryWatcher::new(watch_label, path);
    debug!("Watching {watch_label} for network/shaped updates");
    loop {
        match watch_relevant(&mut watcher) {
            Ok(events) => {
                for event in events {
                    if let Err(err) = dispatch_reload(event) {
                        warn!("Network Devices directory reload dispatch failed: {err}");
                    }
                }
            }
            Err(err) => {
                warn!("{watch_label} watcher returned: {err}");
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn watch_relevant(
    watcher: &mut DirectoryWatcher,
) -> Result<Vec<DirectoryReloadEvent>, WatchedDirectoryError> {
    loop {
        let events = classify_changed_paths(&watcher.watch()?);
        if !events.is_empty() {
            return Ok(events);
        }
    }
}

fn classify_changed_paths(paths: &[PathBuf]) -> Vec<DirectoryReloadEvent> {
    paths
        .iter()
        .filter_map(|path| classify_changed_path(path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn classify_changed_path(path: &Path) -> Option<DirectoryReloadEvent> {
    let file_name = path.file_name()?.to_str()?;
    if is_ignored_filename(file_name) {
        return None;
    }

    match file_name {
        "ShapedDevices.csv"
        | "ShapedDevices.insight.csv"
        | "topology_import.json"
        | "topology_runtime_status.json" => Some(DirectoryReloadEvent::ShapedDevices),
        "network.effective.json" | "network.insight.json" | "network.json" => {
            Some(DirectoryReloadEvent::NetworkJson)
        }
        _ => None,
    }
}

fn is_ignored_filename(file_name: &str) -> bool {
    file_name.ends_with(".lock")
        || file_name.ends_with(".tmp")
        || file_name.ends_with(".swp")
        || file_name.ends_with('~')
}

fn dispatch_reload(event: DirectoryReloadEvent) -> Result<()> {
    match event {
        DirectoryReloadEvent::ShapedDevices => {
            request_reload_shaped_devices("dirwatch:ShapedDevices.csv")
        }
        DirectoryReloadEvent::NetworkJson => request_reload_network_json("dirwatch:network.json"),
    }
}

#[cfg(test)]
mod tests {
    use super::{DirectoryReloadEvent, classify_changed_path, classify_changed_paths};
    use std::path::PathBuf;

    #[test]
    fn topology_runtime_status_triggers_shaped_device_reload() {
        let event = classify_changed_path(&PathBuf::from("/tmp/topology_runtime_status.json"));
        assert_eq!(event, Some(DirectoryReloadEvent::ShapedDevices));
    }

    #[test]
    fn ignored_temp_files_do_not_trigger_reload() {
        let event = classify_changed_path(&PathBuf::from("/tmp/network.json.tmp"));
        assert_eq!(event, None);
    }

    #[test]
    fn changed_paths_are_deduplicated_by_reload_kind() {
        let events = classify_changed_paths(&[
            PathBuf::from("/tmp/network.json"),
            PathBuf::from("/tmp/network.effective.json"),
            PathBuf::from("/tmp/topology_runtime_status.json"),
        ]);
        assert_eq!(
            events,
            vec![
                DirectoryReloadEvent::ShapedDevices,
                DirectoryReloadEvent::NetworkJson
            ]
        );
    }
}
