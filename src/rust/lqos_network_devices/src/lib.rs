//! Shared, centralized access to `network.json` and `ShapedDevices.csv`.
//!
//! This crate provides:
//! - Disk-load helpers used by one-shot tools.
//! - An optional runtime actor + directory watcher used by `lqosd` to keep
//!   in-memory snapshots up to date.

mod actor;
mod catalog;
mod directory_watcher;
mod dynamic;
mod dynamic_store;
mod hash_cache;
mod state;
mod topology;

#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use lqos_config::{ConfigShapedDevices, NetworkJson};
use std::sync::Arc;
use tracing::debug;

pub use catalog::{CircuitRateCaps, ShapedDevicesCatalog};
pub use dynamic::{CircuitObservation, DynamicCircuit};
pub use hash_cache::ShapedDeviceHashCache;
pub use topology::{ResolvedParentNode, resolve_parent_node, resolve_parent_node_reference};

/// Hooks that `lqosd` can provide to run side effects after updates.
pub trait DaemonHooks: Send + Sync + 'static {
    /// Called after a new `ShapedDevices.csv` snapshot is published.
    fn on_shaped_devices_updated(&self);
    /// Called after a new `network.json` snapshot is published.
    fn on_network_json_updated(&self);
    /// Called after one or more dynamic circuits are expired and pruned by the runtime actor.
    ///
    /// Default implementation does nothing so non-daemon callers don't need to handle it.
    fn on_dynamic_circuits_expired(&self, _circuit_ids: &[String]) {}
}

/// Starts the runtime actor and directory watcher used by long-running daemons.
///
/// This function has side effects: it spawns background threads and installs global state.
pub fn start_daemon_mode(hooks: Option<Arc<dyn DaemonHooks>>) -> Result<()> {
    actor::start_actor(hooks)?;
    directory_watcher::start_network_devices_directory_watch()
}

/// Loads `ShapedDevices.csv` from disk.
pub fn load_shaped_devices() -> Result<ConfigShapedDevices> {
    ConfigShapedDevices::load().context("Unable to load ShapedDevices.csv")
}

/// Loads `network.json` from disk.
pub fn load_network_json() -> Result<NetworkJson> {
    NetworkJson::load().context("Unable to load network.json")
}

/// Returns the current in-memory `ShapedDevices.csv` snapshot.
///
/// Note: If the runtime actor has not been started, this returns an empty default snapshot.
pub fn shaped_devices_snapshot() -> Arc<ConfigShapedDevices> {
    state::shaped_devices_snapshot()
}

/// Returns a higher-level shaped-devices catalog snapshot.
///
/// Prefer this API over directly iterating `shaped_devices_snapshot().devices` or separately
/// fetching the hash cache snapshot.
pub fn shaped_devices_catalog() -> ShapedDevicesCatalog {
    state::shaped_devices_catalog()
}

/// Returns the current in-memory shaped-device hash cache snapshot.
pub fn shaped_device_hash_cache_snapshot() -> Arc<ShapedDeviceHashCache> {
    state::shaped_device_hash_cache_snapshot()
}

/// Returns the current in-memory dynamic circuit overlay snapshot.
///
/// Note: This is a runtime-only overlay and does not mutate `ShapedDevices.csv`.
pub fn dynamic_circuits_snapshot() -> Arc<Vec<DynamicCircuit>> {
    state::dynamic_circuits_snapshot()
}

/// Run a closure with a read-only view of the current in-memory `NetworkJson`.
pub fn with_network_json_read<R>(f: impl FnOnce(&NetworkJson) -> R) -> R {
    state::with_network_json_read(f)
}

/// Run a closure with mutable access to the in-memory `NetworkJson`.
///
/// Side effects: this mutates the in-memory snapshot used by `lqosd`.
pub fn with_network_json_write<R>(f: impl FnOnce(&mut NetworkJson) -> R) -> R {
    state::with_network_json_write(f)
}

/// Requests the runtime actor reload `ShapedDevices.csv` from disk.
pub fn request_reload_shaped_devices(reason: &str) -> Result<()> {
    actor::request_reload_shaped_devices(reason)
}

/// Requests the runtime actor reload `network.json` from disk.
pub fn request_reload_network_json(reason: &str) -> Result<()> {
    actor::request_reload_network_json(reason)
}

/// Publishes a caller-provided shaped-devices snapshot through the runtime actor.
pub fn apply_shaped_devices_snapshot(reason: &str, shaped: ConfigShapedDevices) -> Result<()> {
    actor::apply_shaped_devices_snapshot(reason, shaped)
}

/// Reports kernel observations that may indicate dynamic circuit activity.
///
/// This function is not pure: it sends a message to the runtime actor when running.
pub fn report_observations(observations: &[CircuitObservation]) {
    actor::report_observations(observations)
}

/// Creates or updates a runtime-only dynamic circuit overlay entry.
///
/// Side effects:
/// - Updates the in-memory dynamic circuit snapshot.
/// - Writes `dynamic_circuits.json` in the LibreQoS directory.
///
/// This function does **not** mutate `ShapedDevices.csv`.
pub fn upsert_dynamic_circuit(shaped_device: lqos_config::ShapedDevice) -> Result<()> {
    actor::upsert_dynamic_circuit(shaped_device)
}

/// Removes a runtime-only dynamic circuit overlay entry by circuit id.
///
/// Side effects:
/// - Updates the in-memory dynamic circuit snapshot.
/// - Writes `dynamic_circuits.json` in the LibreQoS directory when a circuit is removed.
///
/// This function does **not** mutate `ShapedDevices.csv`.
pub fn remove_dynamic_circuit(circuit_id: &str) -> Result<bool> {
    actor::remove_dynamic_circuit(circuit_id)
}

/// Replaces the in-memory `ShapedDevices.csv` snapshot without involving the runtime actor.
///
/// Side effects:
/// - Updates the shaped-device hash cache snapshot.
/// - Does **not** write to disk.
/// - Does **not** invoke any daemon hooks.
///
/// This is primarily intended for tests and for callers that already hold an `Arc` snapshot.
pub fn swap_shaped_devices_snapshot(
    reason: &str,
    shaped: Arc<ConfigShapedDevices>,
) -> Arc<ConfigShapedDevices> {
    debug!("Swapping shaped-devices snapshot reason={reason}");
    state::swap_shaped_devices_snapshot(shaped)
}
