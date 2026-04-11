use crate::catalog::ShapedDevicesCatalog;
use crate::hash_cache::ShapedDeviceHashCache;
use arc_swap::ArcSwap;
use lqos_config::{ConfigShapedDevices, NetworkJson};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
struct PublishedShapedState {
    shaped: Arc<ConfigShapedDevices>,
    cache: Arc<ShapedDeviceHashCache>,
    generation: u64,
}

static NEXT_SHAPED_GENERATION: AtomicU64 = AtomicU64::new(1);

static SHAPED_STATE: Lazy<ArcSwap<PublishedShapedState>> = Lazy::new(|| {
    ArcSwap::new(Arc::new(PublishedShapedState {
        shaped: Arc::new(ConfigShapedDevices::default()),
        cache: Arc::new(ShapedDeviceHashCache::default()),
        generation: 0,
    }))
});
static NETWORK_JSON: Lazy<RwLock<NetworkJson>> = Lazy::new(|| RwLock::new(NetworkJson::default()));

pub(crate) fn shaped_devices_snapshot() -> Arc<ConfigShapedDevices> {
    SHAPED_STATE.load_full().shaped.clone()
}

pub(crate) fn shaped_device_hash_cache_snapshot() -> Arc<ShapedDeviceHashCache> {
    SHAPED_STATE.load_full().cache.clone()
}

pub(crate) fn shaped_devices_catalog() -> ShapedDevicesCatalog {
    let state = SHAPED_STATE.load_full();
    ShapedDevicesCatalog::new(state.shaped.clone(), state.cache.clone(), state.generation)
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
    let generation = NEXT_SHAPED_GENERATION.fetch_add(1, Ordering::Relaxed);
    let shaped = Arc::new(new_file);
    let cache = Arc::new(ShapedDeviceHashCache::from_devices(&shaped.devices));
    SHAPED_STATE.store(Arc::new(PublishedShapedState {
        shaped,
        cache,
        generation,
    }));
}

pub(crate) fn swap_shaped_devices_snapshot(
    new_snapshot: Arc<ConfigShapedDevices>,
) -> Arc<ConfigShapedDevices> {
    let generation = NEXT_SHAPED_GENERATION.fetch_add(1, Ordering::Relaxed);
    let cache = Arc::new(ShapedDeviceHashCache::from_devices(&new_snapshot.devices));
    let new_state = Arc::new(PublishedShapedState {
        shaped: new_snapshot,
        cache,
        generation,
    });
    let old = SHAPED_STATE.swap(new_state);
    old.shaped.clone()
}

pub(crate) fn publish_network_json(new_file: NetworkJson) {
    let mut writer = NETWORK_JSON.write();
    *writer = new_file;
}
