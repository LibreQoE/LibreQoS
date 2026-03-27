use anyhow::{Result, anyhow};
use lqos_bakery::BakeryCommands;
use lqos_config::{ConfigShapedDevices, ShapedDevice};
use lqos_overrides::{OverrideFile, OverrideLayer, OverrideStore};
use lqos_queue_tracker::QUEUE_STRUCTURE;
use lqos_utils::hash_to_i64;
use std::collections::{BTreeMap, HashMap, HashSet};

pub enum CircuitFallbackOutcome {
    Applied { persisted: bool },
    Cleared { persisted: bool },
    DryRun { action: String },
    Skipped { reason: String },
}

pub struct SiteOverrideUpdate {
    pub site_name: String,
    pub download_bandwidth_mbps: Option<f32>,
    pub upload_bandwidth_mbps: Option<f32>,
}

#[derive(Clone)]
pub struct PersistedCircuitFallback {
    pub sqm_override: String,
    pub devices: Vec<ShapedDevice>,
}

pub fn apply_site_override_updates(updates: &[SiteOverrideUpdate]) -> Result<bool> {
    if updates.is_empty() {
        return Ok(false);
    }

    let mut overrides = OverrideStore::load_layer(OverrideLayer::Stormguard)?;
    let mut changed = false;

    for update in updates {
        let desired = (update.download_bandwidth_mbps, update.upload_bandwidth_mbps);
        if desired == (None, None) {
            let removed = overrides.remove_site_bandwidth_override_count(None, &update.site_name);
            if removed > 0 {
                changed = true;
            }
            continue;
        }

        if current_site_override(&overrides, &update.site_name) == Some(desired) {
            continue;
        }

        overrides.set_site_bandwidth_override(
            None,
            update.site_name.clone(),
            update.download_bandwidth_mbps,
            update.upload_bandwidth_mbps,
        );
        changed = true;
    }

    if !changed {
        return Ok(false);
    }

    OverrideStore::save_layer(OverrideLayer::Stormguard, &overrides)?;
    Ok(true)
}

pub fn apply_circuit_fallback(
    circuit_id: &str,
    sqm_override: &str,
    persist: bool,
    dry_run: bool,
    bakery_sender: crossbeam_channel::Sender<BakeryCommands>,
) -> Result<CircuitFallbackOutcome> {
    let devices = load_devices_for_circuit(circuit_id)?;
    if devices.is_empty() {
        return Ok(CircuitFallbackOutcome::Skipped {
            reason: "No ShapedDevices rows found for circuit.".to_string(),
        });
    }

    if let Some(reason) = conflicting_sqm_owner(&devices)? {
        return Ok(CircuitFallbackOutcome::Skipped { reason });
    }

    if dry_run {
        return Ok(CircuitFallbackOutcome::DryRun {
            action: format!("Would set SQM override to '{sqm_override}'"),
        });
    }

    let persisted = if persist {
        set_devices_sqm_override(&devices, sqm_override)?
    } else {
        false
    };
    apply_circuit_sqm_override_live(circuit_id, &devices, Some(sqm_override), bakery_sender)?;
    Ok(CircuitFallbackOutcome::Applied { persisted })
}

pub fn clear_circuit_fallback(
    circuit_id: &str,
    dry_run: bool,
    bakery_sender: crossbeam_channel::Sender<BakeryCommands>,
) -> Result<CircuitFallbackOutcome> {
    let fallback = load_persisted_circuit_fallbacks()?
        .remove(circuit_id)
        .unwrap_or(PersistedCircuitFallback {
            sqm_override: String::new(),
            devices: load_devices_for_circuit(circuit_id)?,
        });
    if fallback.devices.is_empty() {
        return Ok(CircuitFallbackOutcome::Skipped {
            reason: "No ShapedDevices rows found for circuit.".to_string(),
        });
    }

    if let Some(reason) = conflicting_sqm_owner(&fallback.devices)? {
        return Ok(CircuitFallbackOutcome::Skipped { reason });
    }

    if dry_run {
        return Ok(CircuitFallbackOutcome::DryRun {
            action: "Would clear SQM fallback override".to_string(),
        });
    }

    let device_ids: Vec<String> = fallback
        .devices
        .iter()
        .map(|d| d.device_id.clone())
        .collect();
    let persisted = clear_device_overrides(&device_ids)?;
    apply_circuit_sqm_override_live(circuit_id, &fallback.devices, None, bakery_sender)?;
    Ok(CircuitFallbackOutcome::Cleared { persisted })
}

pub fn load_persisted_circuit_fallbacks() -> Result<HashMap<String, PersistedCircuitFallback>> {
    let overrides = OverrideStore::load_layer(OverrideLayer::Stormguard)?;
    let current_devices = ConfigShapedDevices::load()?.devices;
    Ok(group_circuit_fallbacks(&overrides, &current_devices))
}

fn current_site_override(
    overrides: &OverrideFile,
    site_name: &str,
) -> Option<(Option<f32>, Option<f32>)> {
    overrides
        .network_adjustments()
        .iter()
        .find_map(|adj| match adj {
            lqos_overrides::NetworkAdjustment::AdjustSiteSpeed {
                node_id: _,
                site_name: current,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
            } if current == site_name => Some((*download_bandwidth_mbps, *upload_bandwidth_mbps)),
            _ => None,
        })
}

fn load_devices_for_circuit(circuit_id: &str) -> Result<Vec<ShapedDevice>> {
    let shaped_devices = ConfigShapedDevices::load()?;
    Ok(shaped_devices
        .devices
        .into_iter()
        .filter(|device| device.circuit_id == circuit_id)
        .collect())
}

fn conflicting_sqm_owner(devices: &[ShapedDevice]) -> Result<Option<String>> {
    let device_ids: HashSet<&str> = devices.iter().map(|d| d.device_id.as_str()).collect();

    let operator = OverrideStore::load_layer(OverrideLayer::Operator)?;
    if has_sqm_override_for_device_ids(&operator, &device_ids) {
        return Ok(Some(
            "Operator SQM overrides are present for this circuit.".to_string(),
        ));
    }

    let treeguard = OverrideStore::load_layer(OverrideLayer::Treeguard)?;
    if has_sqm_override_for_device_ids(&treeguard, &device_ids) {
        return Ok(Some(
            "TreeGuard SQM overrides are present for this circuit.".to_string(),
        ));
    }

    Ok(None)
}

fn has_sqm_override_for_device_ids(overrides: &OverrideFile, device_ids: &HashSet<&str>) -> bool {
    overrides.circuit_adjustments().iter().any(|adj| {
        matches!(
            adj,
            lqos_overrides::CircuitAdjustment::DeviceAdjustSqm {
                device_id,
                sqm_override,
            } if device_ids.contains(device_id.as_str())
                && sqm_override
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|token| !token.is_empty())
        )
    }) || overrides.persistent_devices().iter().any(|device| {
        device_ids.contains(device.device_id.as_str())
            && device
                .sqm_override
                .as_deref()
                .is_some_and(|token| !token.trim().is_empty())
    })
}

fn set_devices_sqm_override(base_devices: &[ShapedDevice], sqm_override: &str) -> Result<bool> {
    let mut overrides = OverrideStore::load_layer(OverrideLayer::Stormguard)?;
    let mut changed = false;
    for base_device in base_devices {
        if overrides.set_device_sqm_override_return_changed(
            base_device.device_id.clone(),
            Some(sqm_override.to_string()),
        ) {
            changed = true;
        }
        if overrides.remove_persistent_shaped_device_by_device_count(&base_device.device_id) > 0 {
            changed = true;
        }
    }

    if !changed {
        return Ok(false);
    }

    OverrideStore::save_layer(OverrideLayer::Stormguard, &overrides)?;
    Ok(true)
}

fn clear_device_overrides(device_ids: &[String]) -> Result<bool> {
    let mut overrides = OverrideStore::load_layer(OverrideLayer::Stormguard)?;
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

    OverrideStore::save_layer(OverrideLayer::Stormguard, &overrides)?;
    Ok(true)
}

fn apply_circuit_sqm_override_live(
    circuit_id: &str,
    devices: &[ShapedDevice],
    sqm_override: Option<&str>,
    bakery_sender: crossbeam_channel::Sender<BakeryCommands>,
) -> Result<()> {
    let snapshot = QUEUE_STRUCTURE.load();
    let Some(queues) = snapshot.maybe_queues.as_ref() else {
        return Err(anyhow!("queueingStructure.json not loaded"));
    };

    let mut stack = Vec::new();
    for queue in queues.iter() {
        stack.push(queue);
    }

    let mut found = None;
    while let Some(node) = stack.pop() {
        if node.circuit_id.as_deref() == Some(circuit_id) && node.device_id.is_none() {
            found = Some(node);
            break;
        }

        for child in node.children.iter() {
            stack.push(child);
        }
        for circuit in node.circuits.iter() {
            stack.push(circuit);
        }
        for device in node.devices.iter() {
            stack.push(device);
        }
    }

    let Some(node) = found else {
        return Err(anyhow!(
            "circuit not found in queue structure: {circuit_id}"
        ));
    };

    let class_minor = u16::try_from(node.class_minor)
        .map_err(|_| anyhow!("class_minor too large: {}", node.class_minor))?;
    let class_major = u16::try_from(node.class_major)
        .map_err(|_| anyhow!("class_major too large: {}", node.class_major))?;
    let up_class_major = u16::try_from(node.up_class_major)
        .map_err(|_| anyhow!("up_class_major too large: {}", node.up_class_major))?;

    bakery_sender.send(BakeryCommands::AddCircuit {
        circuit_hash: hash_to_i64(circuit_id),
        parent_class_id: node.parent_class_id,
        up_parent_class_id: node.up_parent_class_id,
        class_minor,
        download_bandwidth_min: node.download_bandwidth_mbps_min as f32,
        upload_bandwidth_min: node.upload_bandwidth_mbps_min as f32,
        download_bandwidth_max: node.download_bandwidth_mbps as f32,
        upload_bandwidth_max: node.upload_bandwidth_mbps as f32,
        class_major,
        up_class_major,
        down_qdisc_handle: None,
        up_qdisc_handle: None,
        ip_addresses: ip_list(devices),
        sqm_override: sqm_override.map(|value| value.to_string()),
    })?;

    Ok(())
}

fn ip_list(devices: &[ShapedDevice]) -> String {
    let mut ips = Vec::new();
    for dev in devices {
        for (ip, prefix) in dev.ipv4.iter() {
            ips.push(format!("{ip}/{prefix}"));
        }
        for (ip, prefix) in dev.ipv6.iter() {
            ips.push(format!("{ip}/{prefix}"));
        }
    }
    ips.sort();
    ips.dedup();
    ips.join(",")
}

fn group_circuit_fallbacks(
    overrides: &OverrideFile,
    current_devices: &[ShapedDevice],
) -> HashMap<String, PersistedCircuitFallback> {
    let devices_by_id: HashMap<&str, &ShapedDevice> = current_devices
        .iter()
        .map(|device| (device.device_id.as_str(), device))
        .collect();
    let mut grouped: BTreeMap<String, PersistedCircuitFallback> = BTreeMap::new();

    for adj in overrides.circuit_adjustments() {
        let lqos_overrides::CircuitAdjustment::DeviceAdjustSqm {
            device_id,
            sqm_override,
        } = adj
        else {
            continue;
        };
        let Some(token) = sqm_override.as_deref().map(str::trim) else {
            continue;
        };
        let Some(device) = devices_by_id.get(device_id.as_str()) else {
            continue;
        };
        if token.is_empty() || device.circuit_id.trim().is_empty() {
            continue;
        }

        let entry =
            grouped
                .entry(device.circuit_id.clone())
                .or_insert_with(|| PersistedCircuitFallback {
                    sqm_override: token.to_string(),
                    devices: Vec::new(),
                });
        if entry.sqm_override != token {
            continue;
        }
        entry.devices.push((*device).clone());
    }

    for device in overrides.persistent_devices() {
        let Some(token) = device.sqm_override.as_deref().map(str::trim) else {
            continue;
        };
        let Some(current_device) = devices_by_id.get(device.device_id.as_str()) else {
            continue;
        };
        if token.is_empty() || current_device.circuit_id.trim().is_empty() {
            continue;
        }

        let entry = grouped
            .entry(current_device.circuit_id.clone())
            .or_insert_with(|| PersistedCircuitFallback {
                sqm_override: token.to_string(),
                devices: Vec::new(),
            });

        if entry.sqm_override != token {
            continue;
        }
        if entry
            .devices
            .iter()
            .any(|existing| existing.device_id == current_device.device_id)
        {
            continue;
        }
        entry.devices.push((*current_device).clone());
    }
    grouped.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_device(device_id: &str, circuit_id: &str) -> ShapedDevice {
        ShapedDevice {
            device_id: device_id.to_string(),
            circuit_id: circuit_id.to_string(),
            comment: "operator-comment".to_string(),
            download_max_mbps: 123.0,
            ..Default::default()
        }
    }

    #[test]
    fn group_circuit_fallbacks_groups_by_circuit_and_token() {
        let mut d1 = sample_device("dev1", "c1");
        d1.sqm_override = Some("fq_codel".to_string());
        let mut d2 = sample_device("dev2", "c1");
        d2.sqm_override = Some("fq_codel".to_string());
        let mut d3 = sample_device("dev3", "c2");
        d3.sqm_override = Some("cake".to_string());

        let mut overrides = OverrideFile::default();
        assert!(overrides.set_device_sqm_override_return_changed(
            "dev1".to_string(),
            Some("fq_codel".to_string())
        ));
        assert!(overrides.set_device_sqm_override_return_changed(
            "dev2".to_string(),
            Some("fq_codel".to_string())
        ));
        assert!(
            overrides.set_device_sqm_override_return_changed(
                "dev3".to_string(),
                Some("cake".to_string())
            )
        );

        let grouped = group_circuit_fallbacks(&overrides, &[d1, d2, d3]);
        assert_eq!(grouped.len(), 2);
        assert_eq!(
            grouped
                .get("c1")
                .expect("c1 circuit fallback should be grouped")
                .sqm_override,
            "fq_codel"
        );
        assert_eq!(
            grouped
                .get("c1")
                .expect("c1 circuit fallback should be grouped")
                .devices
                .len(),
            2
        );
        assert_eq!(
            grouped
                .get("c2")
                .expect("c2 circuit fallback should be grouped")
                .sqm_override,
            "cake"
        );
    }

    #[test]
    fn group_circuit_fallbacks_ignores_blank_or_mixed_tokens() {
        let mut d1 = sample_device("dev1", "c1");
        d1.sqm_override = Some("fq_codel".to_string());
        let mut d2 = sample_device("dev2", "c1");
        d2.sqm_override = Some("cake".to_string());
        let mut d3 = sample_device("dev3", "c2");
        d3.sqm_override = Some(" ".to_string());

        let mut overrides = OverrideFile::default();
        assert!(overrides.set_device_sqm_override_return_changed(
            "dev1".to_string(),
            Some("fq_codel".to_string())
        ));
        assert!(
            overrides.set_device_sqm_override_return_changed(
                "dev2".to_string(),
                Some("cake".to_string())
            )
        );
        assert!(
            overrides
                .set_device_sqm_override_return_changed("dev3".to_string(), Some(" ".to_string()))
        );

        let grouped = group_circuit_fallbacks(&overrides, &[d1, d2, d3]);
        assert_eq!(
            grouped
                .get("c1")
                .expect("c1 should remain grouped when one token differs")
                .devices
                .len(),
            1
        );
        assert!(!grouped.contains_key("c2"));
    }
}
