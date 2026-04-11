use crate::{request_reload_network_json, request_reload_shaped_devices};
use anyhow::Result;
use lqos_utils::directory_watcher::{DirectoryWatcher, WatchedDirectoryError};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tracing::{debug, error, warn};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum DirectoryReloadEvent {
    ShapedDevices,
    NetworkJson,
}

pub(crate) fn start_network_devices_directory_watch() -> Result<()> {
    std::thread::Builder::new()
        .name("Network Devices Dir Watcher".to_string())
        .spawn(|| {
            let mut watcher = match NetworkDevicesDirectoryWatcher::new() {
                Ok(watcher) => watcher,
                Err(err) => {
                    error!("Failed to start Network Devices directory watcher: {err}");
                    return;
                }
            };
            debug!("Watching LibreQoS config directory for network/shaped updates");
            loop {
                match watcher.watch_relevant() {
                    Ok(events) => {
                        for event in events {
                            if let Err(err) = dispatch_reload(event) {
                                warn!("Network Devices directory reload dispatch failed: {err}");
                            }
                        }
                    }
                    Err(err) => {
                        warn!("Network Devices directory watcher returned: {err}");
                    }
                }
            }
        })?;
    Ok(())
}

struct NetworkDevicesDirectoryWatcher {
    inner: DirectoryWatcher,
}

impl NetworkDevicesDirectoryWatcher {
    fn new() -> Result<Self> {
        let config = lqos_config::load_config()?;
        let path = PathBuf::from(&config.lqos_directory);
        Ok(Self {
            inner: DirectoryWatcher::new("Network Devices Directory", path),
        })
    }

    fn watch(&mut self) -> Result<Vec<PathBuf>, WatchedDirectoryError> {
        self.inner.watch()
    }

    fn watch_relevant(&mut self) -> Result<Vec<DirectoryReloadEvent>, WatchedDirectoryError> {
        loop {
            let events = classify_changed_paths(&self.watch()?);
            if !events.is_empty() {
                return Ok(events);
            }
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
        "ShapedDevices.csv" | "ShapedDevices.insight.csv" => {
            Some(DirectoryReloadEvent::ShapedDevices)
        }
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
