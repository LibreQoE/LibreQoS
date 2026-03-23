//! Persistence helpers for TreeGuard.
//!
//! This module writes TreeGuard-owned changes to `lqos_overrides.treeguard.json`.

use crate::treeguard::TreeguardError;
use lqos_overrides::{OverrideLayer, OverrideStore};

/// Removes any node-virtualization overrides for a `network.json` node.
///
/// This function is not pure: it reads and writes `lqos_overrides.treeguard.json`.
///
/// Returns `Ok(true)` if the file was changed, `Ok(false)` if no change was needed.
pub(crate) fn clear_node_virtual(node_name: &str) -> Result<bool, TreeguardError> {
    let mut overrides = OverrideStore::load_layer(OverrideLayer::Treeguard).map_err(|e| {
        TreeguardError::OverridesLoad {
            details: e.to_string(),
        }
    })?;

    let removed = overrides.remove_network_node_virtual_by_name_count(node_name);
    if removed == 0 {
        return Ok(false);
    }

    OverrideStore::save_layer(OverrideLayer::Treeguard, &overrides).map_err(|e| {
        TreeguardError::OverridesSave {
            details: e.to_string(),
        }
    })?;
    Ok(true)
}

/// Persists a per-device SQM override token for a list of devices.
///
/// This function is not pure: it reads and writes `lqos_overrides.treeguard.json`.
///
/// Returns `Ok(true)` if the file was changed, `Ok(false)` if no change was needed.
pub(crate) fn set_devices_sqm_override(
    device_ids: &[String],
    sqm_override: &str,
) -> Result<bool, TreeguardError> {
    let mut overrides = OverrideStore::load_layer(OverrideLayer::Treeguard).map_err(|e| {
        TreeguardError::OverridesLoad {
            details: e.to_string(),
        }
    })?;

    let mut changed = false;
    for device_id in device_ids {
        if overrides.set_device_sqm_override_return_changed(
            device_id.clone(),
            Some(sqm_override.to_string()),
        ) {
            changed = true;
        }
        if overrides.remove_persistent_shaped_device_by_device_count(device_id) > 0 {
            changed = true;
        }
    }

    if !changed {
        return Ok(false);
    }

    OverrideStore::save_layer(OverrideLayer::Treeguard, &overrides).map_err(|e| {
        TreeguardError::OverridesSave {
            details: e.to_string(),
        }
    })?;
    Ok(true)
}

/// Removes any persisted SQM override entries for a list of device IDs.
///
/// This function is not pure: it reads and writes `lqos_overrides.treeguard.json`.
///
/// Returns `Ok(true)` if the file was changed, `Ok(false)` if no change was needed.
pub(crate) fn clear_device_overrides(device_ids: &[String]) -> Result<bool, TreeguardError> {
    let mut overrides = OverrideStore::load_layer(OverrideLayer::Treeguard).map_err(|e| {
        TreeguardError::OverridesLoad {
            details: e.to_string(),
        }
    })?;

    let mut removed_any = false;
    for device_id in device_ids {
        let removed_adjustments = overrides.remove_device_sqm_override_by_device_count(device_id);
        let removed_devices = overrides.remove_persistent_shaped_device_by_device_count(device_id);
        if removed_adjustments > 0 || removed_devices > 0 {
            removed_any = true;
        }
    }

    if !removed_any {
        return Ok(false);
    }

    OverrideStore::save_layer(OverrideLayer::Treeguard, &overrides).map_err(|e| {
        TreeguardError::OverridesSave {
            details: e.to_string(),
        }
    })?;
    Ok(true)
}
