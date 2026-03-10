//! Persistence helpers for TreeGuard.
//!
//! This module writes TreeGuard-owned changes to `lqos_overrides.treeguard.json`.

use crate::treeguard::TreeguardError;
use lqos_config::ShapedDevice;
use lqos_overrides::{NetworkAdjustment, OverrideFile, OverrideLayer, OverrideStore};

/// Returns the current virtual override value for `node_name`, if present.
///
/// This function is pure: it does not perform I/O and has no side effects.
fn current_node_virtual_override(overrides: &OverrideFile, node_name: &str) -> Option<bool> {
    overrides
        .network_adjustments()
        .iter()
        .find_map(|adj| match adj {
            NetworkAdjustment::SetNodeVirtual {
                node_name: n,
                virtual_node,
            } if n == node_name => Some(*virtual_node),
            _ => None,
        })
}

/// Sets (adds or replaces) a node-virtualization override for a `network.json` node.
///
/// This function is not pure: it reads and writes `lqos_overrides.treeguard.json`.
///
/// Returns `Ok(true)` if the file was changed, `Ok(false)` if no change was needed.
pub(crate) fn set_node_virtual(
    node_name: &str,
    virtual_node: bool,
) -> Result<bool, TreeguardError> {
    let mut overrides =
        OverrideStore::load_layer(OverrideLayer::Treeguard).map_err(|e| TreeguardError::OverridesLoad {
        details: e.to_string(),
    })?;

    if current_node_virtual_override(&overrides, node_name) == Some(virtual_node) {
        return Ok(false);
    }

    overrides.set_network_node_virtual(node_name.to_string(), virtual_node);
    OverrideStore::save_layer(OverrideLayer::Treeguard, &overrides).map_err(|e| {
        TreeguardError::OverridesSave {
            details: e.to_string(),
        }
    })?;
    Ok(true)
}

/// Removes any node-virtualization overrides for a `network.json` node.
///
/// This function is not pure: it reads and writes `lqos_overrides.treeguard.json`.
///
/// Returns `Ok(true)` if the file was changed, `Ok(false)` if no change was needed.
pub(crate) fn clear_node_virtual(node_name: &str) -> Result<bool, TreeguardError> {
    let mut overrides =
        OverrideStore::load_layer(OverrideLayer::Treeguard).map_err(|e| TreeguardError::OverridesLoad {
        details: e.to_string(),
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

/// Persists a per-device SQM override token for a list of devices by storing persistent shaped
/// device overlays.
///
/// This function is not pure: it reads and writes `lqos_overrides.treeguard.json`.
///
/// Returns `Ok(true)` if the file was changed, `Ok(false)` if no change was needed.
pub(crate) fn set_devices_sqm_override(
    base_devices: &[ShapedDevice],
    sqm_override: &str,
) -> Result<bool, TreeguardError> {
    let mut overrides =
        OverrideStore::load_layer(OverrideLayer::Treeguard).map_err(|e| TreeguardError::OverridesLoad {
        details: e.to_string(),
    })?;

    let mut changed = false;
    for base_device in base_devices {
        let mut device = base_device.clone();
        device.sqm_override = Some(sqm_override.to_string());
        if overrides.add_persistent_shaped_device_return_changed(device) {
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

/// Removes any persistent shaped device overlay entries for a list of device IDs.
///
/// This function is not pure: it reads and writes `lqos_overrides.treeguard.json`.
///
/// Returns `Ok(true)` if the file was changed, `Ok(false)` if no change was needed.
pub(crate) fn clear_device_overrides(device_ids: &[String]) -> Result<bool, TreeguardError> {
    let mut overrides =
        OverrideStore::load_layer(OverrideLayer::Treeguard).map_err(|e| TreeguardError::OverridesLoad {
        details: e.to_string(),
    })?;

    let mut removed_any = false;
    for device_id in device_ids {
        let removed = overrides.remove_persistent_shaped_device_by_device_count(device_id);
        if removed > 0 {
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
