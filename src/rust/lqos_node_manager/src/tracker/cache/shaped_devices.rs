use lazy_static::*;
use lqos_bus::IpStats;
use lqos_config::ConfigShapedDevices;
use parking_lot::RwLock;

lazy_static! {
    /// Global storage of the shaped devices csv data.
    /// Updated by the file system watcher whenever
    /// the underlying file changes.
    pub static ref SHAPED_DEVICES : RwLock<ConfigShapedDevices> = RwLock::new(ConfigShapedDevices::load().unwrap());
}

lazy_static! {
    /// Global storage of the shaped devices csv data.
    /// Updated by the file system watcher whenever
    /// the underlying file changes.
    pub static ref UNKNOWN_DEVICES : RwLock<Vec<IpStats>> = RwLock::new(Vec::new());
}