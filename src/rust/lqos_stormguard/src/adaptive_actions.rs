use anyhow::{Result, anyhow};
use lqos_config::ShapedDevice;
use lqos_overrides::{NetworkAdjustment, OverrideFile, OverrideLayer, OverrideStore};
use std::collections::HashSet;
use tracing::warn;

#[derive(Debug)]
pub enum CircuitFallbackOutcome {
    Applied { persisted: bool },
    Cleared { persisted: bool },
    DryRun { action: String },
    Skipped { reason: String },
}

pub struct SiteOverrideUpdate {
    /// Stable canonical node id, when the watched site resolved one. Rung-3 1a:
    /// the clamp is written keyed by this id so a later site rename cannot
    /// orphan it; `site_name` remains the display/legacy fallback.
    pub node_id: Option<String>,
    pub site_name: String,
    pub download_bandwidth_mbps: Option<f32>,
    pub upload_bandwidth_mbps: Option<f32>,
}

pub fn apply_site_override_updates(
    updates: &[SiteOverrideUpdate],
    ttl_seconds: u64,
) -> Result<bool> {
    if updates.is_empty() {
        return Ok(false);
    }

    let mut overrides = OverrideStore::load_layer(OverrideLayer::Stormguard)?;
    let mut changed = false;
    let expires_at_unix = unix_time_secs().saturating_add(ttl_seconds);

    for update in updates {
        let desired = (update.download_bandwidth_mbps, update.upload_bandwidth_mbps);
        if desired == (None, None) {
            let removed = overrides.remove_site_bandwidth_override_count(
                update.node_id.as_deref(),
                &update.site_name,
            );
            if removed > 0 {
                changed = true;
            }
            continue;
        }

        // K1b §4.6 pin 1: refresh the dead-man's-switch TTL on every
        // reasserting pass. The expiry is part of the entry, so an
        // otherwise-identical clamp still rewrites (content change →
        // source-generation hash → republish).
        // Rung-3 1a: write keyed by the stable node id when known.
        if overrides.set_site_bandwidth_override_with_expiry(
            update.node_id.clone(),
            update.site_name.clone(),
            update.download_bandwidth_mbps,
            update.upload_bandwidth_mbps,
            Some(expires_at_unix),
        ) {
            changed = true;
        }
    }

    if !changed {
        return Ok(false);
    }

    OverrideStore::save_layer(OverrideLayer::Stormguard, &overrides)?;
    Ok(true)
}

/// K1b §4.6 pin 1: drops StormGuard site-speed entries whose TTL has lapsed,
/// writing the layer back in ONE RMW so the source-generation hash flips and
/// the topology manager republishes the un-clamped rates. Returns `true` when
/// the layer changed.
///
/// Side effects: mutates and persists the StormGuard override layer.
pub fn sweep_lapsed_site_overrides(now_unix: u64) -> Result<bool> {
    let mut overrides = OverrideStore::load_layer(OverrideLayer::Stormguard)?;
    let removed = overrides.remove_lapsed_site_speed_overrides(now_unix);
    if removed.is_empty() {
        return Ok(false);
    }
    for site_name in &removed {
        warn!(
            "StormGuard clamp for site '{site_name}' lapsed without reassertion; dropping the override and republishing"
        );
    }
    OverrideStore::save_layer(OverrideLayer::Stormguard, &overrides)?;
    Ok(true)
}

fn unix_time_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// K1b §4.2 operator supremacy: true when an operator `AdjustSiteSpeed`
/// override already claims this site. StormGuard refuses up-front to clamp
/// such a site (loud WARN at the call site), rather than writing a layer entry
/// the operator-first merge would silently drop.
///
/// Rung-3 1a: the join is id-first — a stable `node_id` match claims the site
/// across renames — with the operator entry's display `site_name` as the
/// fallback for legacy id-less operator entries.
pub fn operator_owns_site_speed(node_id: Option<&str>, site_name: &str) -> Result<bool> {
    let operator = OverrideStore::load_layer(OverrideLayer::Operator)?;
    Ok(operator_adjustment_claims(
        operator.network_adjustments(),
        node_id,
        site_name,
    ))
}

/// Pure id-first / name-fallback claim check for operator `AdjustSiteSpeed`
/// entries (rung-3 1a).
fn operator_adjustment_claims(
    adjustments: &[NetworkAdjustment],
    node_id: Option<&str>,
    site_name: &str,
) -> bool {
    let node_id = node_id.map(str::trim).filter(|node_id| !node_id.is_empty());
    adjustments.iter().any(|adj| {
        matches!(
            adj,
            NetworkAdjustment::AdjustSiteSpeed {
                node_id: current_id,
                site_name: current_name,
                ..
            } if current_name == site_name
                || (node_id.is_some() && current_id.as_deref() == node_id)
        )
    })
}

/// Removes every adaptive override owned by StormGuard.
///
/// Side effects: replaces the persisted StormGuard override layer with an empty layer.
pub fn clear_stormguard_override_layer() -> Result<()> {
    OverrideStore::save_layer(OverrideLayer::Stormguard, &OverrideFile::default())
}

/// K1b ph2 §9.3 (LayerApply strategy): the BRIDGE-mode circuit fallback —
/// write the StormGuard layer entry (with the §9.2 TTL) and wait for the
/// bridge to apply the republished transaction.
pub async fn apply_circuit_fallback_layer(
    circuit_id: &str,
    sqm_override: &str,
    dry_run: bool,
    ttl_seconds: u64,
    publish_wait_seconds: u64,
    bus_tx: crate::bus::StormguardBusSender,
) -> Result<CircuitFallbackOutcome> {
    let devices = load_devices_for_circuit(circuit_id)?;
    if devices.is_empty() {
        return Ok(CircuitFallbackOutcome::Skipped {
            reason: "No ShapedDevices rows found for circuit.".to_string(),
        });
    }
    if let Some(reason) = conflicting_sqm_owner(&devices, circuit_id)? {
        return Ok(CircuitFallbackOutcome::Skipped { reason });
    }
    // §9.1-1: the bridge cannot instantiate fq_codel — reject it loudly.
    if let Err(error) = validate_circuit_aqm_token(sqm_override) {
        return Ok(CircuitFallbackOutcome::Skipped {
            reason: format!("Unsupported circuit AQM token: {error}"),
        });
    }
    if dry_run {
        return Ok(CircuitFallbackOutcome::DryRun {
            action: format!("Would set circuit AQM to '{sqm_override}'"),
        });
    }

    let snapshot = OverrideStore::load_layer(OverrideLayer::Stormguard).ok();
    let expires_at_unix = unix_time_secs().saturating_add(ttl_seconds);
    let changed = set_circuit_aqm_override(circuit_id, Some(sqm_override), Some(expires_at_unix))?;
    if changed
        && let Err(error) =
            crate::site_state::publish_clamp_and_wait(&bus_tx, publish_wait_seconds).await
    {
        if let Some(snapshot) = &snapshot
            && let Err(restore_error) = crate::site_state::restore_override_snapshot(
                snapshot,
                "after a rejected StormGuard circuit AQM fallback",
            )
        {
            warn!(
                "StormGuard could not restore overrides after a rejected circuit fallback: {restore_error}"
            );
        }
        return Err(error);
    }
    Ok(CircuitFallbackOutcome::Applied { persisted: true })
}

/// K1b ph2 §9.3 (LayerApply): restore-by-deletion — drop the layer entry and
/// wait for the republished transaction.
pub async fn clear_circuit_fallback_layer(
    circuit_id: &str,
    dry_run: bool,
    publish_wait_seconds: u64,
    bus_tx: crate::bus::StormguardBusSender,
) -> Result<CircuitFallbackOutcome> {
    let devices = load_devices_for_circuit(circuit_id)?;
    if devices.is_empty() {
        return Ok(CircuitFallbackOutcome::Skipped {
            reason: "No ShapedDevices rows found for circuit.".to_string(),
        });
    }
    if let Some(reason) = conflicting_sqm_owner(&devices, circuit_id)? {
        return Ok(CircuitFallbackOutcome::Skipped { reason });
    }
    if dry_run {
        return Ok(CircuitFallbackOutcome::DryRun {
            action: "Would clear circuit AQM fallback".to_string(),
        });
    }
    let snapshot = OverrideStore::load_layer(OverrideLayer::Stormguard).ok();
    let changed = clear_circuit_aqm_override(circuit_id)?;
    if changed
        && let Err(error) =
            crate::site_state::publish_clamp_and_wait(&bus_tx, publish_wait_seconds).await
    {
        if let Some(snapshot) = &snapshot
            && let Err(restore_error) = crate::site_state::restore_override_snapshot(
                snapshot,
                "after a rejected StormGuard circuit AQM clear",
            )
        {
            warn!(
                "StormGuard could not restore overrides after a rejected circuit clear: {restore_error}"
            );
        }
        return Err(error);
    }
    Ok(CircuitFallbackOutcome::Cleared { persisted: true })
}

fn set_circuit_aqm_override(
    circuit_id: &str,
    aqm_token: Option<&str>,
    expires_at_unix: Option<u64>,
) -> Result<bool> {
    let mut overrides = OverrideStore::load_layer(OverrideLayer::Stormguard)?;
    if !overrides.set_circuit_aqm_return_changed(
        circuit_id.to_string(),
        aqm_token.map(str::to_string),
        expires_at_unix,
    ) {
        return Ok(false);
    }
    OverrideStore::save_layer(OverrideLayer::Stormguard, &overrides)?;
    Ok(true)
}

fn clear_circuit_aqm_override(circuit_id: &str) -> Result<bool> {
    let mut overrides = OverrideStore::load_layer(OverrideLayer::Stormguard)?;
    if overrides.remove_circuit_aqm_by_circuit_count(circuit_id) == 0 {
        return Ok(false);
    }
    OverrideStore::save_layer(OverrideLayer::Stormguard, &overrides)?;
    Ok(true)
}

fn load_devices_for_circuit(circuit_id: &str) -> Result<Vec<ShapedDevice>> {
    let shaped_devices = lqos_network_devices::load_shaped_devices()?;
    Ok(shaped_devices
        .devices
        .into_iter()
        .filter(|device| device.circuit_id == circuit_id)
        .collect())
}

fn conflicting_sqm_owner(devices: &[ShapedDevice], circuit_id: &str) -> Result<Option<String>> {
    let device_ids: HashSet<&str> = devices.iter().map(|d| d.device_id.as_str()).collect();
    let source_devices = lqos_network_devices::load_source_shaped_devices()?.devices;
    if has_source_sqm_override(&device_ids, &source_devices) {
        return Ok(Some(
            "The circuit already has a source-configured SQM override.".to_string(),
        ));
    }

    let operator = OverrideStore::load_layer(OverrideLayer::Operator)?;
    if has_sqm_override_for_device_ids(&operator, &device_ids)
        || has_circuit_aqm_for_circuit(&operator, circuit_id)
    {
        return Ok(Some(
            "Operator SQM overrides are present for this circuit.".to_string(),
        ));
    }

 // TreeGuard is retiring: the treeguard precedence arm collapses
    // to stormguard > operator, so its ownership check is dropped here.
    Ok(None)
}

/// K1b ph2 §9.3: a circuit-level AQM entry in an operator/treeguard layer claims
/// the circuit (operator supremacy, the §4.2 rule applied to SQM). A `none`
/// token is not an ownership claim.
fn has_circuit_aqm_for_circuit(overrides: &OverrideFile, circuit_id: &str) -> bool {
    overrides.circuit_adjustments().iter().any(|adj| {
        matches!(
            adj,
            lqos_overrides::CircuitAdjustment::CircuitAdjustAqm {
                circuit_id: current,
                aqm_token,
                ..
            } if current == circuit_id && aqm_token_claims(aqm_token)
        )
    })
}

fn aqm_token_claims(token: &str) -> bool {
    let trimmed = token.trim();
    !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("none")
}

/// K1b ph2 §9.1-1: the bridge can only instantiate `none` or `cake[...]`.
/// `fq_codel` compiles but the bridge rejects it (live ignore / boot FATAL), so
/// the StormGuard write path must refuse it loudly (the LayerApply bridge
/// strategy). `fq_codel` remains valid for the kernel-mode strategy path.
pub fn validate_circuit_aqm_token(token: &str) -> Result<()> {
    let trimmed = token.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return Ok(());
    }
    // Mirror `aqm_from_override`'s acceptance: any `cake`-prefixed token
    // (the `[diffserv4]`/`rtt<N>` qualifiers are parsed downstream).
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("cake") {
        return Ok(());
    }
    if lower == "fq_codel" {
        return Err(anyhow!(
            "fq_codel is not supported by the bridge; the effective circuit AQM grammar is none | cake"
        ));
    }
    Err(anyhow!(
        "unsupported circuit AQM token '{trimmed}'; the effective grammar is none | cake"
    ))
}

fn has_source_sqm_override(device_ids: &HashSet<&str>, source_devices: &[ShapedDevice]) -> bool {
    source_devices.iter().any(|device| {
        device_ids.contains(device.device_id.as_str())
            && device
                .sqm_override
                .as_deref()
                .is_some_and(|token| !token.trim().is_empty())
    })
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
    fn source_sqm_change_is_visible_while_effective_device_has_stormguard_token() {
        let mut effective = sample_device("dev1", "c1");
        effective.sqm_override = Some("cake".to_string());
        let mut source = sample_device("dev1", "c1");
        source.sqm_override = Some("fq_codel".to_string());
        let device_ids = HashSet::from([effective.device_id.as_str()]);

        assert!(has_source_sqm_override(&device_ids, &[source]));
    }

    #[test]
    fn validate_circuit_aqm_token_accepts_none_and_cake_only() {
        // §9.1-1: the bridge can only instantiate none | cake; fq_codel is a
        // loud reject even though the compiler can express it.
        for ok in ["", "none", "NONE", "cake", "cake diffserv4 rtt20"] {
            assert!(
                validate_circuit_aqm_token(ok).is_ok(),
                "{ok:?} should be accepted"
            );
        }
        for bad in ["fq_codel", "pfifo", "sfq"] {
            assert!(
                validate_circuit_aqm_token(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn lapsed_circuit_aqm_entries_are_swept() {
        let mut overrides = OverrideFile::default();
        overrides.add_circuit_adjustment(lqos_overrides::CircuitAdjustment::CircuitAdjustAqm {
            circuit_id: "c1".to_string(),
            aqm_token: "cake".to_string(),
            expires_at_unix: Some(1),
        });
        overrides.add_circuit_adjustment(lqos_overrides::CircuitAdjustment::CircuitAdjustAqm {
            circuit_id: "c2".to_string(),
            aqm_token: "cake".to_string(),
            expires_at_unix: None,
        });
        let removed = overrides.remove_lapsed_circuit_aqm_overrides(100);
        assert_eq!(removed, vec!["c1".to_string()]);
        assert!(!has_circuit_aqm_for_circuit(&overrides, "c1"));
        assert!(has_circuit_aqm_for_circuit(&overrides, "c2"));
    }

    #[test]
    fn circuit_aqm_entry_claims_the_circuit() {
        let mut overrides = OverrideFile::default();
        overrides.add_circuit_adjustment(lqos_overrides::CircuitAdjustment::CircuitAdjustAqm {
            circuit_id: "c1".to_string(),
            aqm_token: "cake".to_string(),
            expires_at_unix: None,
        });
        assert!(has_circuit_aqm_for_circuit(&overrides, "c1"));
        assert!(!has_circuit_aqm_for_circuit(&overrides, "c2"));
    }

    #[test]
    fn circuit_aqm_none_token_is_not_an_ownership_claim() {
        let mut overrides = OverrideFile::default();
        assert!(overrides.set_circuit_aqm_return_changed(
            "c1".to_string(),
            Some("none".to_string()),
            None
        ));
        assert!(
            !has_circuit_aqm_for_circuit(&overrides, "c1"),
            "a none token must not claim the circuit"
        );
    }

    #[test]
    fn operator_site_speed_claim_joins_by_id_with_name_fallback() {
        let id_carrying = vec![NetworkAdjustment::AdjustSiteSpeed {
            node_id: Some("node-1".to_string()),
            site_name: "old-name".to_string(),
            download_bandwidth_mbps: Some(100.0),
            upload_bandwidth_mbps: None,
            expires_at_unix: None,
        }];
        // Id-first: the operator entry claims the site even after a rename.
        assert!(operator_adjustment_claims(
            &id_carrying,
            Some("node-1"),
            "new-name"
        ));
        assert!(!operator_adjustment_claims(
            &id_carrying,
            Some("node-2"),
            "new-name"
        ));

        let name_only = vec![NetworkAdjustment::AdjustSiteSpeed {
            node_id: None,
            site_name: "legacy-site".to_string(),
            download_bandwidth_mbps: Some(100.0),
            upload_bandwidth_mbps: None,
            expires_at_unix: None,
        }];
        // Legacy fallback: a name-only operator entry still claims by name.
        assert!(operator_adjustment_claims(
            &name_only,
            Some("node-9"),
            "legacy-site"
        ));
        assert!(!operator_adjustment_claims(
            &name_only,
            Some("node-9"),
            "other-site"
        ));
    }
}
