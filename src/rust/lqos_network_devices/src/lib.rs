//! Shared, centralized access to `network.json` and `ShapedDevices.csv`.
//!
//! This crate provides:
//! - Disk-load helpers used by one-shot tools.
//! - An optional runtime actor + directory watcher used by `lqosd` to keep
//!   in-memory snapshots up to date.

mod actor;
mod catalog;
mod combined_catalog;
mod dataplane_tree;
mod directory_watcher;
mod dynamic;
mod dynamic_store;
mod hash_cache;
mod runtime_inputs;
mod state;
mod topology;

#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use lqos_config::{
    Config, ConfigShapedDevices, NetworkJson, TopologyShapingInputsFile,
};
use lqos_topology_compile::TopologyImportFile;
use std::sync::Arc;
use tracing::debug;

pub use catalog::{CircuitRateCaps, ShapedDevicesCatalog};
pub use combined_catalog::NetworkDevicesCatalog;
pub use dynamic::{CircuitObservation, DynamicCircuit};
pub use hash_cache::ShapedDeviceHashCache;
pub use topology::{
    ParentNodeLookup, ResolvedParentNode, resolve_parent_node, resolve_parent_node_reference,
    resolve_parent_node_reference_in_nodes,
};

/// Provenance for caller-provided shaped-device snapshots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShapedDevicesSnapshotProvenance {
    /// Snapshot came from an external caller that already constructed a complete payload.
    ExternalSnapshot,
}

#[derive(Clone, Debug)]
pub(crate) enum LoadedShapedDevicesSource {
    RuntimeTopology {
        shaping_generation: String,
    },
    TopologyImport,
}

pub(crate) struct LoadedShapedDevices {
    pub(crate) shaped: ConfigShapedDevices,
    pub(crate) source: LoadedShapedDevicesSource,
}

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
    /// Called when the runtime actor promotes an unknown IP into a dynamic circuit overlay entry.
    ///
    /// Default implementation does nothing so non-daemon callers don't need to handle it.
    fn on_unknown_ip_promoted(&self, _shaped_device: &lqos_config::ShapedDevice) {}
}

/// Starts the runtime actor and directory watcher used by long-running daemons.
///
/// This function has side effects: it spawns background threads and installs global state.
pub fn start_daemon_mode(hooks: Option<Arc<dyn DaemonHooks>>) -> Result<()> {
    actor::start_actor(hooks)?;
    directory_watcher::start_network_devices_directory_watch()
}

/// Loads the currently active runtime shaping inputs for integration-backed topologies.
///
/// Returns `Ok(None)` when integration ingress is disabled or when no ready runtime shaping
/// inputs have been published yet.
pub fn load_runtime_shaping_inputs_for_config(
    config: &Config,
) -> Result<Option<TopologyShapingInputsFile>> {
    // Phase C S4: the compiled import is the ONE source, so ready runtime
    // inputs are always the first choice (no integration-ingress mode gate).
    runtime_inputs::load_ready_runtime_shaping_inputs(config)
}

fn load_shaped_devices_for_config(config: &lqos_config::Config) -> Result<LoadedShapedDevices> {
    if let Some(shaping_inputs) = load_runtime_shaping_inputs_for_config(config)? {
        let shaped_devices = runtime_inputs::shaped_devices_from_runtime_inputs(&shaping_inputs);
        if !shaped_devices.devices.is_empty() {
            return Ok(LoadedShapedDevices {
                shaped: shaped_devices,
                source: LoadedShapedDevicesSource::RuntimeTopology {
                    shaping_generation: shaping_inputs.shaping_generation,
                },
            });
        }
        debug!(
            "Active runtime shaping inputs contained 0 shaped devices; checking other sources"
        );
    }
    if !lqos_config::topology_import_has_shaped_devices(config) {
        // T5 rung 1 / F10: the zero-CSV identity gate — before keeping the
        // world empty, try the E2 operator view: its circuit nodes carry
        // devices INLINE, so identity renders with no ShapedDevices.csv.
        if let Ok(Some(view)) = runtime_inputs::load_operator_view(config) {
            let shaped_devices = runtime_inputs::shaped_devices_from_operator_view(&view);
            if !shaped_devices.devices.is_empty() {
                return Ok(LoadedShapedDevices {
                    shaped: shaped_devices,
                    source: LoadedShapedDevicesSource::RuntimeTopology {
                        shaping_generation: view.source_generation,
                    },
                });
            }
        }
        debug!(
            "topology_import.json is missing or contains 0 shaped devices; keeping the world empty until topology is published"
        );
        return Ok(LoadedShapedDevices {
            shaped: ConfigShapedDevices::default(),
            source: LoadedShapedDevicesSource::TopologyImport,
        });
    }
    match lqos_topology_compile::TopologyImportFile::load(config) {
        Ok(Some(topology_import)) => {
            let shaped_devices = topology_import.into_imported_bundle().shaped_devices;
            if !shaped_devices.devices.is_empty() {
                return Ok(LoadedShapedDevices {
                    shaped: shaped_devices,
                    source: LoadedShapedDevicesSource::TopologyImport,
                });
            }
            debug!(
                "topology_import.json advertised shaped devices but loaded empty; keeping the world empty until topology is published"
            );
        }
        Ok(None) => {
            debug!(
                "topology_import.json advertised shaped devices but loader returned no bundle; keeping the world empty until topology is published"
            );
        }
        Err(err) => {
            debug!(
                "Unable to load topology_import.json ({err}); keeping the world empty until topology is published"
            );
        }
    }
    Ok(LoadedShapedDevices {
        shaped: ConfigShapedDevices::default(),
        source: LoadedShapedDevicesSource::TopologyImport,
    })
}

fn load_source_shaped_devices_for_config(config: &Config) -> Result<ConfigShapedDevices> {
    // Phase C S4: the compiled import is the ONE source (manual included).
    Ok(TopologyImportFile::load(config)?
        .map(|topology_import| topology_import.into_imported_bundle().shaped_devices)
        .unwrap_or_default())
}

/// Loads shaped-device configuration from disk.
///
/// When integration-backed topology ingress is enabled, this prefers ready runtime shaping inputs,
/// then `topology_import.json`, then an empty snapshot until the topology runtime publishes.
/// Manual mode loads `ShapedDevices.csv`.
pub fn load_shaped_devices() -> Result<ConfigShapedDevices> {
    Ok(load_shaped_devices_with_source()?.shaped)
}

/// Loads the operator or integration source shaped-device rows without runtime overrides.
///
/// Integration-ingress mode reads `topology_import.json`; manual mode reads
/// `ShapedDevices.csv`. This deliberately bypasses ready runtime shaping inputs so callers can
/// detect source settings hidden by an effective adaptive override.
///
/// Side effects: reads the configured source file from disk.
pub fn load_source_shaped_devices() -> Result<ConfigShapedDevices> {
    let config = lqos_config::load_config().context("Unable to load /etc/lqos.conf")?;
    load_source_shaped_devices_for_config(config.as_ref())
}

pub(crate) fn load_shaped_devices_with_source() -> Result<LoadedShapedDevices> {
    let config = lqos_config::load_config().context("Unable to load /etc/lqos.conf")?;
    load_shaped_devices_for_config(config.as_ref())
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

/// Returns a combined catalog snapshot for both shaped devices and dynamic circuits.
pub fn network_devices_catalog() -> NetworkDevicesCatalog {
    NetworkDevicesCatalog::from_snapshots(
        state::shaped_devices_catalog(),
        state::dynamic_circuits_snapshot(),
    )
}

/// Returns the canonical count of unique static or dynamic circuits with a valid IP mapping.
pub fn mapped_circuit_count() -> usize {
    let shaped = state::shaped_devices_snapshot();
    let dynamic = state::dynamic_circuits_snapshot();
    combined_catalog::mapped_circuit_count_for_devices(
        shaped
            .devices
            .iter()
            .chain(dynamic.iter().map(|circuit| &circuit.shaped)),
    )
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

/// Requests the runtime actor repopulate the dashboard tree from the
/// E2 operator view (`topology_operator_view.json`, T5 rung 1). A
/// no-op when the named legacy `network.json` tree mode is enabled.
pub fn request_reload_dataplane_tree(reason: &str) -> Result<()> {
    actor::request_reload_dataplane_tree(reason)
}

/// Publishes a caller-provided shaped-devices snapshot through the runtime actor.
pub fn apply_shaped_devices_snapshot(
    reason: &str,
    shaped: ConfigShapedDevices,
    provenance: ShapedDevicesSnapshotProvenance,
) -> Result<()> {
    actor::apply_shaped_devices_snapshot(reason, shaped, provenance)
}

/// Reports kernel observations that may indicate dynamic circuit activity.
///
/// This function is not pure: it sends a message to the runtime actor when running.
pub fn report_observations(observations: &[CircuitObservation]) {
    actor::report_observations(observations)
}

/// K1 §1.4: reports the bridge's active dynamic-circuit ids — the liveness
/// backstop for the RADIUS-interim-stall class. Only matching dynamic
/// circuits have their `last_seen` refreshed; the kernel observation feed
/// remains a separate (DPDK-inert) arm.
///
/// This function is not pure: it sends a message to the runtime actor.
pub fn report_bridge_activity(circuit_ids: &[String]) {
    actor::report_bridge_activity(circuit_ids)
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

/// Dynamic-ranges H2: replaces any dynamic circuit covering the same device
/// IP(s) with `shaped_device` in ONE RMW (remove-by-IP + upsert-by-id). Used by
/// the promotion scanner and the RADIUS in-place upgrade so a session-derived
/// id cannot mint a duplicate row + Map.
///
/// This function does **not** mutate `ShapedDevices.csv`.
pub fn replace_dynamic_circuit_by_ip(shaped_device: lqos_config::ShapedDevice) -> Result<()> {
    actor::replace_dynamic_circuit_by_ip(shaped_device)
}

/// Dynamic-ranges §7: reports per-IP activity for in-range, catalog-unresolved
/// hosts from the lqosd bridge analytics to the actor's promotion scanner. The
/// actor owns the sustained-activity window, the cap, and the store write.
pub fn report_range_host_activity(range_id: &str, samples: &[(std::net::IpAddr, u64, u64)]) {
    actor::report_range_host_activity(range_id, samples)
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
