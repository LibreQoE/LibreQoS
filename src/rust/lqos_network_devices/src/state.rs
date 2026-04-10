use crate::hash_cache::ShapedDeviceHashCache;
use arc_swap::ArcSwap;
use lqos_config::{ConfigShapedDevices, NetworkJson};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::Arc;

static SHAPED_DEVICES: Lazy<ArcSwap<ConfigShapedDevices>> =
    Lazy::new(|| ArcSwap::new(Arc::new(ConfigShapedDevices::default())));
static SHAPED_DEVICE_HASH_CACHE: Lazy<ArcSwap<ShapedDeviceHashCache>> =
    Lazy::new(|| ArcSwap::new(Arc::new(ShapedDeviceHashCache::default())));
static NETWORK_JSON: Lazy<RwLock<NetworkJson>> = Lazy::new(|| RwLock::new(NetworkJson::default()));

pub(crate) fn shaped_devices_snapshot() -> Arc<ConfigShapedDevices> {
    SHAPED_DEVICES.load_full()
}

pub(crate) fn shaped_device_hash_cache_snapshot() -> Arc<ShapedDeviceHashCache> {
    SHAPED_DEVICE_HASH_CACHE.load_full()
}

pub(crate) fn with_network_json_read<R>(f: impl FnOnce(&NetworkJson) -> R) -> R {
    let reader = NETWORK_JSON.read();
    f(&reader)
}

pub(crate) fn with_network_json_write<R>(f: impl FnOnce(&mut NetworkJson) -> R) -> R {
    let mut writer = NETWORK_JSON.write();
    f(&mut writer)
}

pub(crate) fn publish_shaped_devices(new_file: ConfigShapedDevices) {
    let cache = ShapedDeviceHashCache::from_devices(&new_file.devices);
    SHAPED_DEVICES.store(Arc::new(new_file));
    SHAPED_DEVICE_HASH_CACHE.store(Arc::new(cache));
}

pub(crate) fn swap_shaped_devices_snapshot(
    new_snapshot: Arc<ConfigShapedDevices>,
) -> Arc<ConfigShapedDevices> {
    let cache = ShapedDeviceHashCache::from_devices(&new_snapshot.devices);
    let old = SHAPED_DEVICES.swap(new_snapshot);
    SHAPED_DEVICE_HASH_CACHE.store(Arc::new(cache));
    old
}

pub(crate) fn publish_network_json(new_file: NetworkJson) {
    let mut writer = NETWORK_JSON.write();
    *writer = new_file;
}
