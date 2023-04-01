use lqos_config::ConfigShapedDevices;
use once_cell::sync::Lazy;
use std::sync::RwLock;

/// Global storage of the shaped devices csv data.
/// Updated by the file system watcher whenever
/// the underlying file changes.
pub static SHAPED_DEVICES: Lazy<RwLock<ConfigShapedDevices>> =
  Lazy::new(|| RwLock::new(ConfigShapedDevices::default()));