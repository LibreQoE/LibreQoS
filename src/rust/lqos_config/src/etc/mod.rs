//! Manages the `/etc/lqos.conf` file.

mod etclqos_migration;

use self::migration::migrate_if_needed;
pub use self::v15::Config;
pub use self::v15::{DEFAULT_ANALYTICS_SOCKET, DataplaneBridgeConfig};
use arc_swap::ArcSwap;
pub use etclqos_migration::*;
use once_cell::sync::Lazy;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;
use toml_edit::{DocumentMut, Item, Table, value};
use tracing::{debug, error, info, warn};

mod migration;
mod python_migration;
#[cfg(test)]
pub mod test_data;
mod v15;
pub use v15::{
    BridgeConfig, DynamicCircuitRangeRule, DynamicCircuitsConfig, IntegrationConfig,
    LocalApiKeyConfig, MAX_LOCAL_API_KEYS, MikrotikIpv6Config, PowerConfig, PowerProfile, QueueMode,
    RadiusAccountingClient,
    RadiusAccountingConfig, RadiusClientSource, RadiusDynamicCircuitApplicationConfig,
    RadiusFallbackSpeedProfile, RadiusSharedSecretSource, RateProfileValidationError,
    RttThresholds, SingleInterfaceConfig, SslConfig, StormguardConfig, StormguardStrategy,
    MaintenanceWindowConfig, TopologyConfig, TopologySource, TopologySourceConflict,
    TopologySourceOrigin, TopologySourceRuntime, Tunables, normalize_external_hostname,
    validate_rate_profile_mbps, validate_topology_selection,
};

static CONFIG: Lazy<ArcSwap<Option<Arc<Config>>>> = Lazy::new(|| ArcSwap::from_pointee(None));

/// The canonical v3 configuration path (`/etc/libreqos/lqos.conf`). The legacy
/// v2 path `/etc/lqos.conf` is migrated here by the upgrade (the daemon unit
/// pins `LQOS_CONFIG` to this path).
pub const DEFAULT_CONFIG_PATH: &str = "/etc/libreqos/lqos.conf";

/// Resolve the configuration path: `LQOS_CONFIG` when set, else
/// [`DEFAULT_CONFIG_PATH`]. THE single resolver for read AND write, so the
/// daemon and the setup flow cannot diverge (the split-brain the first
/// repo-served upgrade exposed).
pub fn config_path() -> String {
    std::env::var("LQOS_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string())
}

/// Load the configuration from the resolved path.
pub fn load_config() -> Result<Arc<Config>, LibreQoSConfigError> {
    // Fast path - just load the Arc
    if let Some(config) = CONFIG.load().as_ref().as_ref().cloned() {
        return Ok(config);
    }

    // Config wasn't cached (or a previous load failed). Attempt to load again so
    // transient problems (e.g. config created after process start) can recover.
    let config = actually_load_from_disk()?;
    CONFIG.store(Some(config.clone()).into());
    Ok(config)
}

/// Clears the in-process cached configuration so the next `load_config()` reads from disk again.
///
/// This function has side effects: it mutates the process-global config cache.
#[doc(hidden)]
pub fn clear_cached_config() {
    CONFIG.store(None.into());
}

fn uisp_capacity_defaults_migration_stamp_path(config_path: &str) -> String {
    format!("{config_path}.uisp_capacity_defaults_migrated")
}

fn topology_compile_mode_migration_stamp_path(config_path: &str) -> String {
    format!("{config_path}.topology_compile_mode_migrated")
}

fn enabled_table_string(
    doc: &DocumentMut,
    table_name: &str,
    enabled_key: &str,
    value_key: &str,
) -> Option<String> {
    let table = doc.get(table_name)?.as_table_like()?;
    if !table.get(enabled_key)?.as_bool().unwrap_or(false) {
        return None;
    }
    let raw = table.get(value_key)?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    crate::etc::v15::normalize_topology_compile_mode(raw).map(ToString::to_string)
}

fn maybe_migrate_topology_compile_mode(
    config_location: &str,
    raw: String,
) -> Result<String, LibreQoSConfigError> {
    let stamp_path = topology_compile_mode_migration_stamp_path(config_location);
    if Path::new(&stamp_path).exists() {
        return Ok(raw);
    }

    let mut doc: DocumentMut = raw.parse().map_err(|e| LibreQoSConfigError::ParseError {
        path: config_location.to_string(),
        details: format!("Error parsing config: {e}"),
    })?;

    if let Some(existing) = doc
        .get("topology")
        .and_then(|section| section.as_table_like())
        .and_then(|section| section.get("compile_mode"))
        .and_then(|mode| mode.as_str())
        .and_then(crate::etc::v15::normalize_topology_compile_mode)
    {
        std::fs::write(&stamp_path, format!("{existing}\n")).map_err(|e| {
            LibreQoSConfigError::CannotWrite {
                path: stamp_path.clone(),
                source: e,
            }
        })?;
        return Ok(raw);
    }

    let uisp_mode = enabled_table_string(&doc, "uisp_integration", "enable_uisp", "strategy");
    let splynx_mode = enabled_table_string(&doc, "splynx_integration", "enable_splynx", "strategy");

    let selected = match (uisp_mode.as_deref(), splynx_mode.as_deref()) {
        (Some(uisp), Some(splynx)) if uisp == splynx => Some(uisp.to_string()),
        (Some(_), Some(_)) => None,
        (Some(uisp), None) => Some(uisp.to_string()),
        (None, Some(splynx)) => Some(splynx.to_string()),
        (None, None) => None,
    };

    let Some(selected) = selected else {
        if uisp_mode.is_some() && splynx_mode.is_some() {
            info!(
                "Topology compile mode migration skipped because UISP and Splynx legacy strategies differ."
            );
        }
        std::fs::write(&stamp_path, b"skipped\n").map_err(|e| {
            LibreQoSConfigError::CannotWrite {
                path: stamp_path,
                source: e,
            }
        })?;
        return Ok(raw);
    };

    if !doc.as_table().contains_key("topology") {
        doc["topology"] = Item::Table(Table::new());
    }
    doc["topology"]["compile_mode"] = value(selected.as_str());
    let migrated = doc.to_string();

    let config_path = Path::new(config_location);
    if config_path.exists() {
        let backup_path = format!("{config_location}.topology_compile_mode_backup");
        std::fs::copy(config_path, &backup_path).map_err(|e| LibreQoSConfigError::CannotWrite {
            path: backup_path,
            source: e,
        })?;
    }

    std::fs::write(config_path, &migrated).map_err(|e| LibreQoSConfigError::CannotWrite {
        path: config_location.to_string(),
        source: e,
    })?;
    std::fs::write(&stamp_path, format!("{selected}\n")).map_err(|e| {
        LibreQoSConfigError::CannotWrite {
            path: stamp_path,
            source: e,
        }
    })?;

    info!("Topology compile mode was automatically migrated to '{selected}'.");
    Ok(migrated)
}

const TOPOLOGY_SOURCE_FLAGS: [(TopologySource, &str, &str); 7] = [
    (TopologySource::Uisp, "uisp_integration", "enable_uisp"),
    (TopologySource::Splynx, "splynx_integration", "enable_splynx"),
    (TopologySource::Netzur, "netzur_integration", "enable_netzur"),
    (TopologySource::Visp, "visp_integration", "enable_visp"),
    (TopologySource::Sonar, "sonar_integration", "enable_sonar"),
    (
        TopologySource::Powercode,
        "powercode_integration",
        "enable_powercode",
    ),
    (TopologySource::Wispgate, "wispgate_integration", "enable_wispgate"),
];

fn topology_source_migration_stamp_path(config_path: &str) -> String {
    format!("{config_path}.topology_source_migrated")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParsedTopologySource {
    Absent,
    Empty,
    Value(TopologySource),
}

fn read_topology_source(
    doc: &DocumentMut,
    config_location: &str,
) -> Result<ParsedTopologySource, LibreQoSConfigError> {
    let Some(item) = doc
        .get("topology")
        .and_then(|section| section.as_table_like())
        .and_then(|section| section.get("source"))
    else {
        return Ok(ParsedTopologySource::Absent);
    };
    let Some(raw) = item.as_str() else {
        return Err(LibreQoSConfigError::MigrationFailed {
            path: config_location.to_string(),
            details: format!(
                "topology.source must be a string; accepted values: {}",
                TopologySource::accepted_list()
            ),
        });
    };
    if raw.trim().is_empty() {
        return Ok(ParsedTopologySource::Empty);
    }
    TopologySource::from_value(raw)
        .map(ParsedTopologySource::Value)
        .ok_or_else(|| LibreQoSConfigError::MigrationFailed {
            path: config_location.to_string(),
            details: format!(
                "unknown topology source '{}'; accepted values: {}",
                raw.trim(),
                TopologySource::accepted_list()
            ),
        })
}

fn read_topology_compile_mode(
    doc: &DocumentMut,
    config_location: &str,
) -> Result<String, LibreQoSConfigError> {
    let Some(item) = doc
        .get("topology")
        .and_then(|section| section.as_table_like())
        .and_then(|section| section.get("compile_mode"))
    else {
        return Ok(String::new());
    };
    item.as_str()
        .map(ToString::to_string)
        .ok_or_else(|| LibreQoSConfigError::MigrationFailed {
            path: config_location.to_string(),
            details: "topology.compile_mode must be a string".to_string(),
        })
}

fn topology_enabled_flags(doc: &DocumentMut) -> Vec<(TopologySource, &'static str)> {
    TOPOLOGY_SOURCE_FLAGS
        .iter()
        .filter_map(|&(source, section, key)| {
            let enabled = doc
                .get(section)
                .and_then(|table| table.as_table_like())
                .and_then(|table| table.get(key))
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            enabled.then_some((source, key))
        })
        .collect()
}

fn topology_conflict_message(source: Option<TopologySource>, flags: &[String]) -> String {
    let flag_list = flags.join(" and ");
    match source {
        Some(source) => format!(
            "topology source conflict: [topology].source is \"{}\" but {} enabled; set [topology].source or disable all but one",
            source.as_str(),
            flag_list
        ),
        None => format!(
            "topology source conflict: {} are enabled; set [topology].source or disable all but one",
            flag_list
        ),
    }
}

fn write_topology_source_stamp(
    config_location: &str,
    stamp: &str,
) -> Result<(), LibreQoSConfigError> {
    let stamp_path = topology_source_migration_stamp_path(config_location);
    std::fs::write(&stamp_path, format!("{stamp}\n")).map_err(|e| {
        LibreQoSConfigError::CannotWrite {
            path: stamp_path,
            source: e,
        }
    })
}

/// D1/D2 source migration + stamp. Returns the (possibly rewritten) raw config
/// and the runtime conflict/origin state. A source conflict is returned as
/// `Ok` with `conflict` set (D11 degraded); unknown/missing-after-stamp are
/// fail-stop errors.
fn maybe_migrate_topology_source(
    config_location: &str,
    raw: String,
) -> Result<(String, TopologySourceRuntime), LibreQoSConfigError> {
    let stamp_path = topology_source_migration_stamp_path(config_location);
    let stamp_exists = Path::new(&stamp_path).exists();
    let mut doc: DocumentMut = raw.parse().map_err(|e| LibreQoSConfigError::ParseError {
        path: config_location.to_string(),
        details: format!("Error parsing config: {e}"),
    })?;
    let parsed_source = read_topology_source(&doc, config_location)?;
    let compile_mode = read_topology_compile_mode(&doc, config_location)?;
    let enabled = topology_enabled_flags(&doc);
    let mut runtime = TopologySourceRuntime {
        conflict: None,
        origin: TopologySourceOrigin::Missing,
    };

    match parsed_source {
        ParsedTopologySource::Value(source) => {
            validate_topology_selection(Some(source), &compile_mode).map_err(|details| {
                LibreQoSConfigError::MigrationFailed {
                    path: config_location.to_string(),
                    details,
                }
            })?;

            let conflicting: Vec<String> = enabled
                .iter()
                .filter(|(enabled_source, _)| *enabled_source != source)
                .map(|(_, key)| (*key).to_string())
                .collect();
            if !conflicting.is_empty() {
                runtime.conflict = Some(TopologySourceConflict {
                    source: Some(source),
                    enabled: conflicting.clone(),
                    message: topology_conflict_message(Some(source), &conflicting),
                });
                runtime.origin = TopologySourceOrigin::Config;
                return Ok((raw, runtime));
            }

            if enabled.iter().any(|(enabled_source, _)| *enabled_source == source) {
                warn!(
                    "Legacy {} is redundant; [topology].source is authoritative.",
                    source.enable_flag_key().unwrap_or("enable_*")
                );
            }
            if !stamp_exists {
                write_topology_source_stamp(
                    config_location,
                    &format!("settled:{}", source.as_str()),
                )?;
            }
            runtime.origin = TopologySourceOrigin::Config;
            Ok((raw, runtime))
        }
        ParsedTopologySource::Empty | ParsedTopologySource::Absent => {
            if stamp_exists {
                return Err(LibreQoSConfigError::MigrationFailed {
                    path: config_location.to_string(),
                    details:
                        "no topology source selected; set [topology].source in lqos.conf".to_string(),
                });
            }

            let selected = if enabled.is_empty() {
                Some(TopologySource::ShapedDevices)
            } else if enabled.len() == 1 {
                Some(enabled[0].0)
            } else {
                None
            };
            let Some(source) = selected else {
                let flags = enabled
                    .iter()
                    .map(|(_, key)| (*key).to_string())
                    .collect::<Vec<_>>();
                runtime.conflict = Some(TopologySourceConflict {
                    source: None,
                    enabled: flags.clone(),
                    message: topology_conflict_message(None, &flags),
                });
                runtime.origin = TopologySourceOrigin::Missing;
                return Ok((raw, runtime));
            };

            validate_topology_selection(Some(source), &compile_mode).map_err(|details| {
                LibreQoSConfigError::MigrationFailed {
                    path: config_location.to_string(),
                    details,
                }
            })?;

            if !doc.as_table().contains_key("topology") {
                doc["topology"] = Item::Table(Table::new());
            }
            doc["topology"]["source"] = value(source.as_str());
            let migrated = doc.to_string();

            let config_path = Path::new(config_location);
            if config_path.exists() {
                let backup_path = format!("{config_location}.topology_source_backup");
                std::fs::copy(config_path, &backup_path).map_err(|e| {
                    LibreQoSConfigError::CannotWrite {
                        path: backup_path,
                        source: e,
                    }
                })?;
            }
            std::fs::write(config_path, &migrated).map_err(|e| {
                LibreQoSConfigError::CannotWrite {
                    path: config_location.to_string(),
                    source: e,
                }
            })?;
            write_topology_source_stamp(
                config_location,
                &format!("migrated:{}", source.as_str()),
            )?;
            info!(
                "Topology source was automatically migrated to '{}'.",
                source.as_str()
            );
            runtime.origin = TopologySourceOrigin::Migrated;
            Ok((migrated, runtime))
        }
    }
}

fn maybe_migrate_uisp_capacity_defaults(
    config_location: &str,
    raw: String,
) -> Result<String, LibreQoSConfigError> {
    let stamp_path = uisp_capacity_defaults_migration_stamp_path(config_location);
    if Path::new(&stamp_path).exists() {
        return Ok(raw);
    }

    let mut doc: DocumentMut = raw.parse().map_err(|e| LibreQoSConfigError::ParseError {
        path: config_location.to_string(),
        details: format!("Error parsing config: {e}"),
    })?;

    if !doc.as_table().contains_key("uisp_integration") {
        doc["uisp_integration"] = Item::Table(Table::new());
    }

    doc["uisp_integration"]["airmax_capacity"] = value(1.0);
    doc["uisp_integration"]["ltu_capacity"] = value(1.0);
    let migrated = doc.to_string();

    if migrated != raw {
        let config_path = Path::new(config_location);
        if config_path.exists() {
            let backup_path = format!("{config_location}.uisp_capacity_defaults_backup");
            std::fs::copy(config_path, &backup_path).map_err(|e| {
                LibreQoSConfigError::CannotWrite {
                    path: backup_path,
                    source: e,
                }
            })?;
        }

        std::fs::write(config_path, &migrated).map_err(|e| LibreQoSConfigError::CannotWrite {
            path: config_location.to_string(),
            source: e,
        })?;

        info!(
            "UISP AirMax and LTU capacity defaults were automatically normalized to 1.0 during upgrade."
        );
    }

    std::fs::write(&stamp_path, b"1.0\n").map_err(|e| LibreQoSConfigError::CannotWrite {
        path: stamp_path,
        source: e,
    })?;

    Ok(migrated)
}

fn default_sqm_migration_stamp_path(config_path: &str) -> String {
    format!("{config_path}.default_sqm_migrated")
}

/// The only default SQM the v3 DPDK pipeline shapes with (the node UI's
/// `SUPPORTED_DEFAULT_SQM`, `queues_hygiene.mjs`).
const V3_DEFAULT_SQM: &str = "cake diffserv4";

/// v2→v3: normalize the on-disk `[queues] default_sqm` to the one value the v3
/// DPDK pipeline supports (`cake diffserv4`).
///
/// A v2 box's `fq_codel` (or the ack-filter CAKE variant, or an empty value)
/// would otherwise compile **every override-less leaf to FIFO** at first boot
/// (`dataplane_compiler::aqm_from_override`: a non-`cake` token ⇒
/// `DataplaneLeafAqm::None`) — a silent drop of the SQM layer, because the UI
/// only normalizes on SAVE. Backup + stamp + LOG. A value already equal to the
/// supported one, or an ABSENT key (the v15 struct default already applies), is
/// left byte-identical.
fn maybe_migrate_default_sqm(
    config_location: &str,
    raw: String,
) -> Result<String, LibreQoSConfigError> {
    let stamp_path = default_sqm_migration_stamp_path(config_location);
    if Path::new(&stamp_path).exists() {
        return Ok(raw);
    }

    let mut doc: DocumentMut = raw.parse().map_err(|e| LibreQoSConfigError::ParseError {
        path: config_location.to_string(),
        details: format!("Error parsing config: {e}"),
    })?;

    let Some(current) = doc
        .get("queues")
        .and_then(|queues| queues.as_table_like())
        .and_then(|queues| queues.get("default_sqm"))
        .and_then(|sqm| sqm.as_str())
        .map(str::trim)
        .map(ToString::to_string)
    else {
        // Absent ⇒ the v15 `QueueConfig` default ("cake diffserv4") already
        // applies; nothing to rewrite.
        std::fs::write(&stamp_path, b"absent\n").map_err(|e| {
            LibreQoSConfigError::CannotWrite {
                path: stamp_path.clone(),
                source: e,
            }
        })?;
        return Ok(raw);
    };

    if current == V3_DEFAULT_SQM {
        std::fs::write(&stamp_path, format!("{current}\n")).map_err(|e| {
            LibreQoSConfigError::CannotWrite {
                path: stamp_path.clone(),
                source: e,
            }
        })?;
        return Ok(raw);
    }

    // An empty value would also FAIL the v15 validation ("default_sqm cannot be
    // empty"), so migrating it is a fix, not just a normalization.
    if !doc.as_table().contains_key("queues") {
        doc["queues"] = Item::Table(Table::new());
    }
    doc["queues"]["default_sqm"] = value(V3_DEFAULT_SQM);
    let migrated = doc.to_string();

    let config_path = Path::new(config_location);
    if config_path.exists() {
        let backup_path = format!("{config_location}.default_sqm_backup");
        std::fs::copy(config_path, &backup_path).map_err(|e| LibreQoSConfigError::CannotWrite {
            path: backup_path,
            source: e,
        })?;
    }
    std::fs::write(config_path, &migrated).map_err(|e| LibreQoSConfigError::CannotWrite {
        path: config_location.to_string(),
        source: e,
    })?;
    std::fs::write(&stamp_path, format!("{V3_DEFAULT_SQM}\n")).map_err(|e| {
        LibreQoSConfigError::CannotWrite {
            path: stamp_path,
            source: e,
        }
    })?;
    info!(
        "Default SQM '{current}' is unsupported by the v3 DPDK pipeline; migrated to '{V3_DEFAULT_SQM}'."
    );
    Ok(migrated)
}

fn bandwidth_dataplane_migration_stamp_path(config_path: &str) -> String {
    format!("{config_path}.bandwidth_dataplane_migrated")
}

/// v2→v3: land the v2 `[queues] downlink/uplink_bandwidth_mbps` in the v3
/// `[dataplane]` single source (`download_mbps`/`upload_mbps`) — card
/// `docs/lqos_conf_single_source_card.md` §3.
///
/// The `[queues]` keys still resolve (the transition fallback in the control
/// plane + the engine), so this is a **consistency** migration, not a bug fix —
/// but it makes a v2 box's real root rate the one the v3 single source carries.
/// Fires only when the `[dataplane]` section is ABSENT, the box is unstamped,
/// and **BOTH** `[queues]` keys are present on disk (the v15 `DataplaneConfig`
/// requires both; inventing a missing side would be wrong). Backup + stamp +
/// LOG; byte-identical otherwise.
fn maybe_migrate_bandwidth_to_dataplane(
    config_location: &str,
    raw: String,
) -> Result<String, LibreQoSConfigError> {
    let stamp_path = bandwidth_dataplane_migration_stamp_path(config_location);
    if Path::new(&stamp_path).exists() {
        return Ok(raw);
    }

    let mut doc: DocumentMut = raw.parse().map_err(|e| LibreQoSConfigError::ParseError {
        path: config_location.to_string(),
        details: format!("Error parsing config: {e}"),
    })?;

    let stamped = |reason: &str| -> Result<(), LibreQoSConfigError> {
        std::fs::write(&stamp_path, format!("{reason}\n")).map_err(|e| {
            LibreQoSConfigError::CannotWrite {
                path: stamp_path.clone(),
                source: e,
            }
        })
    };

    if doc.as_table().contains_key("dataplane") {
        stamped("dataplane-present")?;
        return Ok(raw);
    }
    let queues = doc.get("queues").and_then(|queues| queues.as_table_like());
    // Accept an integer OR a decimal Mbps (the v3 decimal form).
    let read_mbps = |item: Option<&Item>| -> Option<f64> {
        let item = item?;
        item.as_float()
            .or_else(|| item.as_integer().map(|value| value as f64))
    };
    let download = read_mbps(queues.and_then(|queues| queues.get("downlink_bandwidth_mbps")));
    let upload = read_mbps(queues.and_then(|queues| queues.get("uplink_bandwidth_mbps")));
    let (Some(download), Some(upload)) = (download, upload) else {
        // A missing side (or no [queues]) ⇒ the fallback still applies; do not
        // invent a value.
        stamped("queues-bandwidth-incomplete")?;
        return Ok(raw);
    };

    doc["dataplane"] = Item::Table(Table::new());
    doc["dataplane"]["download_mbps"] = value(download);
    doc["dataplane"]["upload_mbps"] = value(upload);
    let migrated = doc.to_string();

    let config_path = Path::new(config_location);
    if config_path.exists() {
        let backup_path = format!("{config_location}.bandwidth_dataplane_backup");
        std::fs::copy(config_path, &backup_path).map_err(|e| LibreQoSConfigError::CannotWrite {
            path: backup_path,
            source: e,
        })?;
    }
    std::fs::write(config_path, &migrated).map_err(|e| LibreQoSConfigError::CannotWrite {
        path: config_location.to_string(),
        source: e,
    })?;
    stamped("migrated")?;
    info!(
        "Migrated [queues] bandwidth {download}/{upload} Mbps into the [dataplane] single source."
    );
    Ok(migrated)
}

/// `lqos-74t.55`: ensure an emitted `[bridge]` table carries the legacy
/// `use_xdp_bridge` key for the **external** (V2) `lqos_api` consumer.
///
/// v3 removed `use_xdp_bridge` as a dead v2 key (`08af2da80`;
/// `docs/config_surface_card.md:217` — it is NOT a v15 `BridgeConfig` field, so
/// serde DROPS it). The downloaded `lqos_api` still deserializes it as a
/// **required** `[bridge]` field, so a preserved (`--force-confold`) or
/// v3-emitted config makes `libreqos-api` crash-loop
/// (`missing field use_xdp_bridge at [bridge]`). We do **not** re-add the v15
/// schema field (the config-surface record documents it is absent; the
/// k3-retirement posture forbids kernel-mode config surfaces); we make the
/// **emitted document** carry the key for the external consumer, as the
/// `lqos-74t.55` acceptance requires.
///
/// Only injects when a `[bridge]` table exists and the key is absent, so an
/// explicit operator value and a `[single_interface]`-only config are left
/// untouched. Returns `true` when the document changed.
fn ensure_bridge_use_xdp_bridge_compat(doc: &mut DocumentMut) -> bool {
    let Some(bridge) = doc.get_mut("bridge").and_then(|item| item.as_table_like_mut()) else {
        return false;
    };
    if bridge.contains_key("use_xdp_bridge") {
        return false;
    }
    bridge.insert("use_xdp_bridge", value(false));
    true
}

/// `lqos-74t.55` load-path emission: rewrite the on-disk config only when the
/// `[bridge]` compat key is absent. No stamp — re-checked on every load so a
/// WebUI `update_config` save (which serializes the v15 struct) cannot strand
/// the key permanently.
fn maybe_migrate_bridge_use_xdp_bridge_compat(
    config_location: &str,
    raw: String,
) -> Result<String, LibreQoSConfigError> {
    let mut doc: DocumentMut = raw.parse().map_err(|e| LibreQoSConfigError::ParseError {
        path: config_location.to_string(),
        details: format!("Error parsing config: {e}"),
    })?;

    if !ensure_bridge_use_xdp_bridge_compat(&mut doc) {
        return Ok(raw);
    }

    let migrated = doc.to_string();
    let config_path = Path::new(config_location);
    if config_path.exists() {
        let backup_path = format!("{config_location}.bridge_use_xdp_bridge_backup");
        std::fs::copy(config_path, &backup_path).map_err(|e| LibreQoSConfigError::CannotWrite {
            path: backup_path,
            source: e,
        })?;
    }

    std::fs::write(config_path, &migrated).map_err(|e| LibreQoSConfigError::CannotWrite {
        path: config_location.to_string(),
        source: e,
    })?;
    info!(
        "Added [bridge] use_xdp_bridge = false for external lqos_api compatibility (lqos-74t.55)."
    );
    Ok(migrated)
}

/// `lqos-74t.55` write-path emission: apply the same compat key to the
/// serialized v15 struct so `update_config` (the WebUI save) preserves it.
fn with_bridge_use_xdp_bridge_compat(serialized: String) -> Result<String, LibreQoSConfigError> {
    let mut doc: DocumentMut = serialized.parse().map_err(|e| {
        error!("Unable to re-parse serialized configuration to TOML: {e:?}");
        LibreQoSConfigError::SerializeError
    })?;
    if ensure_bridge_use_xdp_bridge_compat(&mut doc) {
        Ok(doc.to_string())
    } else {
        Ok(serialized)
    }
}

fn actually_load_from_disk() -> Result<Arc<Config>, LibreQoSConfigError> {
    let config_location = config_path();
    if std::env::var("LQOS_CONFIG").is_ok() {
        info!("Overriding lqos.conf location from environment variable.");
    }
    load_config_from_location(&config_location)
}

fn load_config_from_location(
    config_location: &str,
) -> Result<Arc<Config>, LibreQoSConfigError> {
    debug!("Loading configuration file {config_location}");
    migrate_if_needed(config_location).map_err(|e| {
        error!("Unable to migrate configuration: {:?}", e);
        match &e {
            migration::MigrationError::ReadError(io)
                if io.kind() == std::io::ErrorKind::NotFound =>
            {
                LibreQoSConfigError::NotFound {
                    path: config_location.to_string(),
                }
            }
            _ => LibreQoSConfigError::MigrationFailed {
                path: config_location.to_string(),
                details: e.to_string(),
            },
        }
    })?;

    let raw = std::fs::read_to_string(config_location).map_err(|e| {
        error!("Unable to read {config_location}: {e:?}");
        if e.kind() == std::io::ErrorKind::NotFound {
            LibreQoSConfigError::NotFound {
                path: config_location.to_string(),
            }
        } else {
            LibreQoSConfigError::CannotRead {
                path: config_location.to_string(),
                source: e,
            }
        }
    })?;

    let raw = maybe_migrate_uisp_capacity_defaults(config_location, raw)?;
    // v2→v3: an unsupported `[queues] default_sqm` (e.g. `fq_codel`) would
    // otherwise compile every override-less leaf to FIFO. Runs BEFORE the v15
    // parse (which rejects an empty `default_sqm`).
    let raw = maybe_migrate_default_sqm(config_location, raw)?;
    // v2→v3: land the v2 [queues] root rate in the [dataplane] single source.
    let raw = maybe_migrate_bandwidth_to_dataplane(config_location, raw)?;
    let (raw, topology_source_runtime) = maybe_migrate_topology_source(config_location, raw)?;
    // A degraded source conflict must not let the legacy compile-mode migration
    // introduce a mode the retained source does not support.
    let raw = if topology_source_runtime.conflict.is_some() {
        raw
    } else {
        maybe_migrate_topology_compile_mode(config_location, raw)?
    };
    let raw = maybe_migrate_bridge_use_xdp_bridge_compat(config_location, raw)?;

    let mut final_config = Config::load_from_string(&raw).map_err(|e| {
        error!("Unable to parse {config_location}");
        LibreQoSConfigError::ParseError {
            path: config_location.to_string(),
            details: e,
        }
    })?;

    final_config.topology_source_runtime = topology_source_runtime;

    // Check for environment variable overrides
    if let Ok(lqos_dir) = std::env::var("LQOS_DIRECTORY") {
        final_config.lqos_directory = lqos_dir;
    }
    // Phase D: `load_config` is READ-ONLY. The writey legs (legacy runtime
    // state migration, the stale-topology quarantine, the legacy credentials
    // migration) moved to the explicit `repair_runtime_state` entry, called by
    // the write path (lqosd startup; the scheduler's idempotent repair).
    debug!("Set cached version of config file");
    let new_config = Arc::new(final_config);

    Ok(new_config)
}

/// One shared predicate for the 150 watcher and the 151 routing: true only
/// when the selector is set to `source` and there is no degraded conflict.
pub fn topology_source_is(config: &Config, source: TopologySource) -> bool {
    config.active_topology_source() == Some(source)
}

/// Update the configuration on disk.
pub fn update_config(new_config: &Config) -> Result<(), LibreQoSConfigError> {
    let config_location = config_path();
    let persisted = update_config_at(&config_location, new_config)?;
    // Publish the normalized snapshot only after persistence succeeds. A
    // failed write must not leave callers using credentials that never reached
    // disk and cannot be recovered after restart. A first-run setup config
    // with no source is not cached as `Missing`; the next load performs the D1
    // migration to `shaped_devices`.
    if persisted.topology.source.is_some() {
        CONFIG.store(Some(Arc::new(persisted)).into());
    } else {
        clear_cached_config();
    }
    Ok(())
}

fn update_config_at(
    config_location: &str,
    new_config: &Config,
) -> Result<Config, LibreQoSConfigError> {
    debug!("Updating stored configuration");

    let source = new_config.topology.source;
    let mut config = new_config.clone();
    if let Some(source) = source {
        config.normalize_topology_source_flags();
        validate_topology_selection(Some(source), &config.topology.compile_mode).map_err(
            |details| LibreQoSConfigError::MigrationFailed {
                path: config_location.to_string(),
                details,
            },
        )?;
        config.topology_source_runtime = TopologySourceRuntime {
            conflict: None,
            origin: TopologySourceOrigin::Config,
        };
    } else {
        // First-run setup writes a source-less default; the next load performs
        // the D1 migration. Do not let a stray enabled flag slip through.
        validate_topology_selection(None, &config.topology.compile_mode).map_err(|details| {
            LibreQoSConfigError::MigrationFailed {
                path: config_location.to_string(),
                details,
            }
        })?;
        let any_flag = config.uisp_integration.enable_uisp
            || config.splynx_integration.enable_splynx
            || config.powercode_integration.enable_powercode
            || config.sonar_integration.enable_sonar
            || config
                .netzur_integration
                .as_ref()
                .is_some_and(|integration| integration.enable_netzur)
            || config
                .visp_integration
                .as_ref()
                .is_some_and(|integration| integration.enable_visp)
            || config
                .wispgate_integration
                .as_ref()
                .is_some_and(|integration| integration.enable_wispgate);
        if any_flag {
            return Err(LibreQoSConfigError::MigrationFailed {
                path: config_location.to_string(),
                details:
                    "no topology source selected but a legacy enable_* flag is true; set [topology].source"
                        .to_string(),
            });
        }
        config.topology_source_runtime = TopologySourceRuntime::default();
    }

    // Does the configuration exist?
    let config_path = std::path::PathBuf::from(config_location);
    if config_path.exists() {
        let backup_path = std::path::PathBuf::from(format!("{}.webbackup", config_path.display()));
        std::fs::copy(&config_path, &backup_path).map_err(|e| {
            error!("Unable to create backup configuration: {e:?}");
            LibreQoSConfigError::CannotCopy
        })?;
    }

    // Serialize the new one. `lqos-74t.55`: keep the `[bridge]` compat key so
    // an external lqos_api keeps parsing the emitted config.
    let serialized = toml::to_string_pretty(&config).map_err(|e| {
        error!("Unable to serialize new configuration to TOML: {e:?}");
        LibreQoSConfigError::SerializeError
    })?;
    let serialized = with_bridge_use_xdp_bridge_compat(serialized)?;
    std::fs::write(&config_path, serialized).map_err(|e| {
        error!("Unable to write new configuration: {e:?}");
        LibreQoSConfigError::CannotWrite {
            path: config_path.display().to_string(),
            source: e,
        }
    })?;

    if let Some(source) = source {
        write_topology_source_stamp(
            config_location,
            &format!("settled:{}", source.as_str()),
        )?;
    }

    Ok(config)
}

#[derive(Debug, Error)]
pub enum LibreQoSConfigError {
    #[error(
        "Unable to locate LibreQoS configuration at {path}. Set `LQOS_CONFIG` to override the path."
    )]
    NotFound { path: String },
    #[error("Unable to read LibreQoS configuration at {path}: {source}")]
    CannotRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Configuration migration failed for {path}: {details}")]
    MigrationFailed { path: String, details: String },
    #[error("Unable to parse LibreQoS configuration at {path}: {details}")]
    ParseError { path: String, details: String },
    #[error("Could not backup configuration")]
    CannotCopy,
    #[error("Unable to serialize config")]
    SerializeError,
    #[error("Could not write LibreQoS configuration to {path}: {source}")]
    CannotWrite {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        bandwidth_dataplane_migration_stamp_path, default_sqm_migration_stamp_path,
        load_config_from_location, maybe_migrate_bandwidth_to_dataplane,
        maybe_migrate_bridge_use_xdp_bridge_compat, maybe_migrate_default_sqm,
        maybe_migrate_topology_compile_mode, maybe_migrate_uisp_capacity_defaults,
        topology_compile_mode_migration_stamp_path, topology_source_is,
        topology_source_migration_stamp_path, update_config_at,
        uisp_capacity_defaults_migration_stamp_path, with_bridge_use_xdp_bridge_compat, Config,
        TopologySource, TopologySourceConflict, TopologySourceOrigin,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "lqos_config_uisp_capacity_test_{}_{}",
            std::process::id(),
            nanos
        ))
    }

    fn write_test_config(base_dir: &Path, raw: &str) -> PathBuf {
        fs::create_dir_all(base_dir).expect("should create temp dir");
        let config_path = base_dir.join("lqos.conf");
        fs::write(&config_path, raw).expect("should write test config");
        config_path
    }

    fn path_string(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn uisp_capacity_migration_rewrites_existing_values_and_stamps() {
        let test_dir = unique_test_dir();
        let raw = "[uisp_integration]\nairmax_capacity = 0.65\nltu_capacity = 0.9\n";
        let config_path = write_test_config(&test_dir, raw);

        let config_path_str = path_string(&config_path);
        let migrated = maybe_migrate_uisp_capacity_defaults(&config_path_str, raw.to_string())
            .expect("migration should succeed");

        assert!(migrated.contains("airmax_capacity = 1.0"));
        assert!(migrated.contains("ltu_capacity = 1.0"));
        assert_eq!(
            fs::read_to_string(&config_path).expect("config should be rewritten"),
            migrated
        );
        assert_eq!(
            fs::read_to_string(config_path.with_extension("conf.uisp_capacity_defaults_backup"))
                .expect("backup should exist"),
            raw
        );
        assert!(
            Path::new(&uisp_capacity_defaults_migration_stamp_path(
                &config_path_str
            ))
            .exists()
        );

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    #[test]
    fn uisp_capacity_migration_stamps_already_correct_values_without_rewrite() {
        let test_dir = unique_test_dir();
        let raw = "[uisp_integration]\nairmax_capacity = 1.0\nltu_capacity = 1.0\n";
        let config_path = write_test_config(&test_dir, raw);

        let config_path_str = path_string(&config_path);
        let migrated = maybe_migrate_uisp_capacity_defaults(&config_path_str, raw.to_string())
            .expect("migration should succeed");

        assert_eq!(migrated, raw);
        assert_eq!(
            fs::read_to_string(&config_path).expect("config should remain unchanged"),
            raw
        );
        assert!(
            !config_path
                .with_extension("conf.uisp_capacity_defaults_backup")
                .exists()
        );
        assert!(
            Path::new(&uisp_capacity_defaults_migration_stamp_path(
                &config_path_str
            ))
            .exists()
        );

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    #[test]
    fn uisp_capacity_migration_respects_manual_changes_after_stamp() {
        let test_dir = unique_test_dir();
        let raw = "[uisp_integration]\nairmax_capacity = 0.65\nltu_capacity = 0.9\n";
        let config_path = write_test_config(&test_dir, raw);

        let config_path_str = path_string(&config_path);
        maybe_migrate_uisp_capacity_defaults(&config_path_str, raw.to_string())
            .expect("initial migration should succeed");

        let manual = "[uisp_integration]\nairmax_capacity = 0.72\nltu_capacity = 0.81\n";
        fs::write(&config_path, manual).expect("should simulate operator edit");

        let second = maybe_migrate_uisp_capacity_defaults(&config_path_str, manual.to_string())
            .expect("second migration should skip after stamp");

        assert_eq!(second, manual);
        assert_eq!(
            fs::read_to_string(&config_path).expect("manual edit should remain"),
            manual
        );

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    #[test]
    fn topology_compile_mode_migration_copies_enabled_uisp_strategy() {
        let test_dir = unique_test_dir();
        let raw = "[uisp_integration]\nenable_uisp = true\nstrategy = \"full2\"\n";
        let config_path = write_test_config(&test_dir, raw);

        let config_path_str = path_string(&config_path);
        let migrated = maybe_migrate_topology_compile_mode(&config_path_str, raw.to_string())
            .expect("migration should succeed");

        assert!(migrated.contains("[topology]"));
        assert!(migrated.contains("compile_mode = \"full\""));
        assert!(
            Path::new(&topology_compile_mode_migration_stamp_path(
                &config_path_str
            ))
            .exists()
        );

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    #[test]
    fn topology_compile_mode_migration_copies_enabled_splynx_strategy() {
        let test_dir = unique_test_dir();
        let raw = "[splynx_integration]\nenable_splynx = true\nstrategy = \"ap_site\"\n";
        let config_path = write_test_config(&test_dir, raw);

        let config_path_str = path_string(&config_path);
        let migrated = maybe_migrate_topology_compile_mode(&config_path_str, raw.to_string())
            .expect("migration should succeed");

        assert!(migrated.contains("[topology]"));
        assert!(migrated.contains("compile_mode = \"ap_site\""));
        assert!(
            Path::new(&topology_compile_mode_migration_stamp_path(
                &config_path_str
            ))
            .exists()
        );

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    #[test]
    fn topology_compile_mode_migration_skips_conflicting_legacy_strategies() {
        let test_dir = unique_test_dir();
        let raw = "[uisp_integration]\nenable_uisp = true\nstrategy = \"full\"\n\n[splynx_integration]\nenable_splynx = true\nstrategy = \"ap_only\"\n";
        let config_path = write_test_config(&test_dir, raw);

        let config_path_str = path_string(&config_path);
        let migrated = maybe_migrate_topology_compile_mode(&config_path_str, raw.to_string())
            .expect("migration should succeed");

        assert_eq!(migrated, raw);
        assert!(!migrated.contains("[topology]"));
        assert!(
            Path::new(&topology_compile_mode_migration_stamp_path(
                &config_path_str
            ))
            .exists()
        );

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    /// `lqos-74t.55`: a preserved `[bridge]` without `use_xdp_bridge` gains the
    /// key (DPDK posture `false`) so the external V2 `lqos_api` can parse it.
    #[test]
    fn bridge_use_xdp_bridge_compat_adds_key_to_preserved_bridge_config() {
        let test_dir = unique_test_dir();
        let raw = "version = \"1.5\"\n\n[bridge]\nto_internet = \"eth0\"\nto_network = \"eth1\"\n";
        let config_path = write_test_config(&test_dir, raw);
        let config_path_str = path_string(&config_path);

        let migrated =
            maybe_migrate_bridge_use_xdp_bridge_compat(&config_path_str, raw.to_string())
                .expect("compat emission should succeed");

        assert!(migrated.contains("use_xdp_bridge = false"), "{migrated}");
        assert_eq!(
            fs::read_to_string(&config_path).expect("config should be rewritten"),
            migrated
        );
        assert_eq!(
            fs::read_to_string(config_path.with_extension("conf.bridge_use_xdp_bridge_backup"))
                .expect("backup should exist"),
            raw
        );

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    /// An explicit operator value is preserved (we never clobber it).
    #[test]
    fn bridge_use_xdp_bridge_compat_preserves_explicit_value() {
        let test_dir = unique_test_dir();
        let raw =
            "version = \"1.5\"\n\n[bridge]\nuse_xdp_bridge = true\nto_internet = \"eth0\"\n";
        let config_path = write_test_config(&test_dir, raw);
        let config_path_str = path_string(&config_path);

        let migrated =
            maybe_migrate_bridge_use_xdp_bridge_compat(&config_path_str, raw.to_string())
                .expect("compat emission should succeed");

        assert_eq!(migrated, raw);
        assert!(
            !config_path
                .with_extension("conf.bridge_use_xdp_bridge_backup")
                .exists()
        );

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    /// A `[single_interface]`-only config is left untouched (no `[bridge]`
    /// table is created for the external consumer).
    #[test]
    fn bridge_use_xdp_bridge_compat_ignores_single_interface_only() {
        let test_dir = unique_test_dir();
        let raw = "version = \"1.5\"\n\n[single_interface]\ninterface = \"eth0\"\n\
                   internet_vlan = 2\nnetwork_vlan = 3\n";
        let config_path = write_test_config(&test_dir, raw);
        let config_path_str = path_string(&config_path);

        let migrated =
            maybe_migrate_bridge_use_xdp_bridge_compat(&config_path_str, raw.to_string())
                .expect("compat emission should succeed");

        assert_eq!(migrated, raw);
        assert!(!migrated.contains("use_xdp_bridge"));

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    /// Re-checked on every load: once present, a second pass is a no-op.
    #[test]
    fn bridge_use_xdp_bridge_compat_is_idempotent_on_second_load() {
        let test_dir = unique_test_dir();
        let raw = "version = \"1.5\"\n\n[bridge]\nto_internet = \"eth0\"\nto_network = \"eth1\"\n";
        let config_path = write_test_config(&test_dir, raw);
        let config_path_str = path_string(&config_path);

        let first = maybe_migrate_bridge_use_xdp_bridge_compat(&config_path_str, raw.to_string())
            .expect("first pass should migrate");
        let second =
            maybe_migrate_bridge_use_xdp_bridge_compat(&config_path_str, first.clone())
                .expect("second pass should be a no-op");

        assert_eq!(second, first);
        assert_eq!(
            fs::read_to_string(&config_path).expect("config should remain"),
            first
        );

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    /// The `update_config` (WebUI save) emission path. A struct-serialized
    /// `[bridge]` (the ONLY shape `update_config` can feed — `BridgeConfig` has
    /// no `use_xdp_bridge` field) never carries the key, so the write path
    /// ALWAYS emits the DPDK posture `false`. A struct-serialized
    /// single-interface config has no `[bridge]` table, so nothing is added.
    #[test]
    fn update_config_compat_emission_adds_false_from_struct_serialized_bridge() {
        let serialized = "[bridge]\nto_internet = \"eth0\"\nto_network = \"eth1\"\n".to_string();
        let out = with_bridge_use_xdp_bridge_compat(serialized).expect("should emit");
        assert!(out.contains("use_xdp_bridge = false"), "{out}");

        let single = "[single_interface]\ninterface = \"eth0\"\n".to_string();
        let out2 = with_bridge_use_xdp_bridge_compat(single.clone()).expect("should emit");
        assert_eq!(out2, single);
    }

    /// A v2 `fq_codel` default is migrated to the supported v3 value (with a
    /// backup + stamp) — otherwise every override-less leaf compiles to FIFO.
    #[test]
    fn default_sqm_migration_rewrites_fq_codel_and_backs_up() {
        let test_dir = unique_test_dir();
        let raw = "version = \"1.5\"\n[queues]\ndefault_sqm = \"fq_codel\"\n";
        let config_path = write_test_config(&test_dir, raw);
        let config_path_str = path_string(&config_path);

        let migrated = maybe_migrate_default_sqm(&config_path_str, raw.to_string())
            .expect("migration should succeed");
        assert!(migrated.contains("default_sqm = \"cake diffserv4\""), "{migrated}");
        assert_eq!(fs::read_to_string(&config_path).unwrap(), migrated);
        assert_eq!(
            fs::read_to_string(config_path.with_extension("conf.default_sqm_backup")).unwrap(),
            raw
        );
        assert!(Path::new(&default_sqm_migration_stamp_path(&config_path_str)).exists());

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    /// An empty value would FAIL v15 validation; the migration repairs it.
    #[test]
    fn default_sqm_migration_repairs_an_empty_value() {
        let test_dir = unique_test_dir();
        let raw = "version = \"1.5\"\n[queues]\ndefault_sqm = \"\"\n";
        let config_path = write_test_config(&test_dir, raw);
        let migrated = maybe_migrate_default_sqm(&path_string(&config_path), raw.to_string())
            .expect("migration should succeed");
        assert!(migrated.contains("default_sqm = \"cake diffserv4\""), "{migrated}");
        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    /// The supported value and an ABSENT key are both no-ops (byte-identical).
    #[test]
    fn default_sqm_migration_is_a_noop_when_already_supported_or_absent() {
        for raw in [
            "version = \"1.5\"\n[queues]\ndefault_sqm = \"cake diffserv4\"\n",
            "version = \"1.5\"\n[queues]\nqueue_mode = \"shape\"\n",
        ] {
            let test_dir = unique_test_dir();
            let config_path = write_test_config(&test_dir, raw);
            let out = maybe_migrate_default_sqm(&path_string(&config_path), raw.to_string())
                .expect("migration should succeed");
            assert_eq!(out, raw);
            assert_eq!(fs::read_to_string(&config_path).unwrap(), raw);
            fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
        }
    }

    /// Second pass after the stamp is a no-op even if the value regressed.
    #[test]
    fn default_sqm_migration_respects_the_stamp() {
        let test_dir = unique_test_dir();
        let raw = "version = \"1.5\"\n[queues]\ndefault_sqm = \"fq_codel\"\n";
        let config_path = write_test_config(&test_dir, raw);
        let config_path_str = path_string(&config_path);
        maybe_migrate_default_sqm(&config_path_str, raw.to_string()).expect("first pass");
        let second = maybe_migrate_default_sqm(&config_path_str, raw.to_string())
            .expect("second pass");
        assert_eq!(second, raw);
        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    /// A v2 `[queues]` root rate lands in the `[dataplane]` single source once.
    #[test]
    fn bandwidth_migration_lands_queues_rate_in_dataplane() {
        let test_dir = unique_test_dir();
        let raw = "version = \"1.5\"\n[queues]\ndownlink_bandwidth_mbps = 8000\nuplink_bandwidth_mbps = 5000\n";
        let config_path = write_test_config(&test_dir, raw);
        let config_path_str = path_string(&config_path);

        let migrated = maybe_migrate_bandwidth_to_dataplane(&config_path_str, raw.to_string())
            .expect("migration should succeed");
        assert!(migrated.contains("[dataplane]"), "{migrated}");
        assert!(migrated.contains("download_mbps = 8000"), "{migrated}");
        assert!(migrated.contains("upload_mbps = 5000"), "{migrated}");
        // The [queues] facts are preserved (the transition fallback).
        assert!(migrated.contains("downlink_bandwidth_mbps = 8000"));
        assert_eq!(
            fs::read_to_string(config_path.with_extension("conf.bandwidth_dataplane_backup"))
                .unwrap(),
            raw
        );
        assert!(Path::new(&bandwidth_dataplane_migration_stamp_path(&config_path_str)).exists());

        // Idempotent.
        let second =
            maybe_migrate_bandwidth_to_dataplane(&config_path_str, migrated.clone()).unwrap();
        assert_eq!(second, migrated);

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    /// Already-present `[dataplane]`, or a missing `[queues]` side, is a no-op.
    #[test]
    fn bandwidth_migration_is_a_noop_when_dataplane_present_or_incomplete() {
        for raw in [
            "[dataplane]\ndownload_mbps = 900\nupload_mbps = 450\n[queues]\ndownlink_bandwidth_mbps = 8000\nuplink_bandwidth_mbps = 5000\n",
            "[queues]\ndownlink_bandwidth_mbps = 8000\n",
            "[queues]\nqueue_mode = \"shape\"\n",
        ] {
            let test_dir = unique_test_dir();
            let config_path = write_test_config(&test_dir, raw);
            let out =
                maybe_migrate_bandwidth_to_dataplane(&path_string(&config_path), raw.to_string())
                    .expect("migration should succeed");
            assert_eq!(out, raw);
            assert_eq!(fs::read_to_string(&config_path).unwrap(), raw);
            fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
        }
    }
    fn base_config() -> Config {
        let mut config = Config::default();
        config.topology.compile_mode = "full".to_string();
        config.topology.source = None;
        config.normalize_topology_source_flags();
        config
    }

    fn set_flag(config: &mut Config, source: TopologySource, enabled: bool) {
        use TopologySource::*;
        match source {
            Uisp => config.uisp_integration.enable_uisp = enabled,
            Splynx => config.splynx_integration.enable_splynx = enabled,
            Netzur => {
                config
                    .netzur_integration
                    .get_or_insert_with(Default::default)
                    .enable_netzur = enabled
            }
            Visp => {
                config
                    .visp_integration
                    .get_or_insert_with(Default::default)
                    .enable_visp = enabled
            }
            Sonar => config.sonar_integration.enable_sonar = enabled,
            Powercode => config.powercode_integration.enable_powercode = enabled,
            Wispgate => {
                config
                    .wispgate_integration
                    .get_or_insert_with(Default::default)
                    .enable_wispgate = enabled
            }
            ShapedDevices | LibreqosNetwork => {}
        }
    }

    fn write_model(base_dir: &Path, config: &Config) -> PathBuf {
        let raw = toml::to_string_pretty(config).expect("serialize config");
        write_test_config(base_dir, &raw)
    }

    fn load_model(path: &Path) -> Result<std::sync::Arc<Config>, super::LibreQoSConfigError> {
        load_config_from_location(&path_string(path))
    }

    fn stamp_text(path: &Path) -> Option<String> {
        fs::read_to_string(topology_source_migration_stamp_path(&path_string(path))).ok()
    }

    fn write_raw(base_dir: &Path, raw: &str) -> PathBuf {
        write_test_config(base_dir, raw)
    }

    #[test]
    fn topology_source_migration_matrix_rows_1_to_19() {
        for (index, source) in TopologySource::INTEGRATION_SOURCES
            .iter()
            .copied()
            .enumerate()
        {
            let dir = unique_test_dir().join(format!("row{}", index + 1));
            let mut config = base_config();
            set_flag(&mut config, source, true);
            let path = write_model(&dir, &config);

            let loaded = load_model(&path).expect("single enabled flag should migrate");
            assert_eq!(loaded.topology.source, Some(source));
            assert_eq!(loaded.topology_source_runtime.origin, TopologySourceOrigin::Migrated);
            assert_eq!(
                stamp_text(&path),
                Some(format!("migrated:{}\n", source.as_str()))
            );
            assert!(
                fs::read_to_string(&path)
                    .unwrap()
                    .contains(&format!("source = \"{}\"", source.as_str()))
            );
            fs::remove_dir_all(&dir).expect("cleanup");
        }

        // Row 8: two enabled flags is a loud, no-write conflict; after the fix
        // the next load retries and migrates.
        let dir = unique_test_dir().join("row8");
        let mut config = base_config();
        set_flag(&mut config, TopologySource::Uisp, true);
        set_flag(&mut config, TopologySource::Visp, true);
        let path = write_model(&dir, &config);
        let loaded = load_model(&path).expect("source conflict is degraded, not fail-stop");
        assert_eq!(loaded.topology.source, None);
        assert_eq!(loaded.active_topology_source(), None);
        let conflict = loaded
            .topology_source_runtime
            .conflict
            .as_ref()
            .expect("row 8 conflict");
        assert_eq!(conflict.source, None);
        assert!(conflict.enabled.iter().any(|flag| flag == "enable_uisp"));
        assert!(conflict.enabled.iter().any(|flag| flag == "enable_visp"));
        assert_eq!(loaded.topology_source_runtime.origin, TopologySourceOrigin::Missing);
        assert!(
            !Path::new(&topology_source_migration_stamp_path(&path_string(&path))).exists()
        );
        assert!(!fs::read_to_string(&path).unwrap().contains("source ="));
        set_flag(&mut config, TopologySource::Visp, false);
        write_model(&dir, &config);
        let retried = load_model(&path).expect("fixed flags should migrate");
        assert_eq!(retried.topology.source, Some(TopologySource::Uisp));
        assert_eq!(
            stamp_text(&path),
            Some("migrated:uisp\n".to_string())
        );
        fs::remove_dir_all(&dir).expect("cleanup");

        // Rows 9/10: no flags always migrates to shaped_devices; files are only
        // onboarding state, never a source signal.
        for (name, with_files) in [("row9", true), ("row10", false)] {
            let dir = unique_test_dir().join(name);
            let mut config = base_config();
            let lqos_dir = dir.join("lqos");
            fs::create_dir_all(&lqos_dir).expect("create lqos dir");
            config.lqos_directory = lqos_dir.to_string_lossy().into_owned();
            if with_files {
                fs::write(lqos_dir.join("network.json"), "{}").unwrap();
                fs::write(lqos_dir.join("ShapedDevices.csv"), "").unwrap();
            }
            let path = write_model(&dir, &config);
            let loaded = load_model(&path).expect("D1 shaped_devices default");
            assert_eq!(loaded.topology.source, Some(TopologySource::ShapedDevices));
            assert_eq!(loaded.topology_source_runtime.origin, TopologySourceOrigin::Migrated);
            assert_eq!(
                stamp_text(&path),
                Some("migrated:shaped_devices\n".to_string())
            );
            fs::remove_dir_all(&dir).expect("cleanup");
        }

        // Rows 11/12/17: a source already in the file is settled on first load.
        for (name, source, matching_flag) in [
            ("row11", TopologySource::Visp, false),
            ("row12", TopologySource::Uisp, true),
            ("row17", TopologySource::Splynx, false),
        ] {
            let dir = unique_test_dir().join(name);
            let mut config = base_config();
            config.topology.source = Some(source);
            if matching_flag {
                set_flag(&mut config, source, true);
            }
            let path = write_model(&dir, &config);
            let loaded = load_model(&path).expect("source-set install should settle");
            assert_eq!(loaded.topology.source, Some(source));
            assert_eq!(loaded.topology_source_runtime.origin, TopologySourceOrigin::Config);
            assert_eq!(
                stamp_text(&path),
                Some(format!("settled:{}\n", source.as_str()))
            );
            fs::remove_dir_all(&dir).expect("cleanup");
        }

        // Rows 13/14: a different/extra flag is a loud degraded conflict; the
        // source is retained and no stamp is written.
        for (name, extra) in [
            ("row13", TopologySource::Visp),
            ("row14", TopologySource::Sonar),
        ] {
            let dir = unique_test_dir().join(name);
            let mut config = base_config();
            config.topology.source = Some(TopologySource::Uisp);
            set_flag(&mut config, TopologySource::Uisp, true);
            set_flag(&mut config, extra, true);
            if name == "row14" {
                set_flag(&mut config, TopologySource::Netzur, true);
            }
            let path = write_model(&dir, &config);
            let loaded = load_model(&path).expect("hand-edited conflict is degraded");
            assert_eq!(loaded.topology.source, Some(TopologySource::Uisp));
            assert_eq!(loaded.active_topology_source(), None);
            let conflict = loaded
                .topology_source_runtime
                .conflict
                .as_ref()
                .expect("row 13/14 conflict");
            assert_eq!(conflict.source, Some(TopologySource::Uisp));
            assert!(
                !Path::new(&topology_source_migration_stamp_path(&path_string(&path))).exists()
            );
            fs::remove_dir_all(&dir).expect("cleanup");
        }

        // Row 15a/15b: empty is absent only when unstamped.
        let dir = unique_test_dir().join("row15a");
        let raw = toml::to_string_pretty(&base_config())
            .unwrap()
            .replace("[topology]\n", "[topology]\nsource = \"\"\n");
        let path = write_raw(&dir, &raw);
        let loaded = load_model(&path).expect("unstamped empty migrates");
        assert_eq!(loaded.topology.source, Some(TopologySource::ShapedDevices));
        fs::remove_dir_all(&dir).expect("cleanup");

        let dir = unique_test_dir().join("row15b");
        let path = write_raw(&dir, &raw);
        fs::write(
            topology_source_migration_stamp_path(&path_string(&path)),
            "settled:shaped_devices\n",
        )
        .unwrap();
        let error = load_model(&path).expect_err("stamped empty must fail loud");
        assert!(error.to_string().contains("no topology source selected"), "{error}");
        fs::remove_dir_all(&dir).expect("cleanup");

        // Row 16: unknown source fails with the accepted set.
        let dir = unique_test_dir().join("row16");
        let raw = toml::to_string_pretty(&base_config())
            .unwrap()
            .replace("[topology]\n", "[topology]\nsource = \"nope\"\n");
        let path = write_raw(&dir, &raw);
        let error = load_model(&path).expect_err("unknown source must fail");
        assert!(error.to_string().contains("accepted values"), "{error}");
        assert!(error.to_string().contains("shaped_devices"), "{error}");
        fs::remove_dir_all(&dir).expect("cleanup");

        // Row 18: stamped but missing source is fail-stop.
        let dir = unique_test_dir().join("row18");
        let path = write_model(&dir, &base_config());
        fs::write(
            topology_source_migration_stamp_path(&path_string(&path)),
            "migrated:uisp\n",
        )
        .unwrap();
        let error = load_model(&path).expect_err("stamped missing source must fail");
        assert!(error.to_string().contains("no topology source selected"), "{error}");
        fs::remove_dir_all(&dir).expect("cleanup");

        // Row 19: an unsupported source x mode pair fails without a source stamp.
        let dir = unique_test_dir().join("row19");
        let mut config = base_config();
        config.topology.source = Some(TopologySource::Visp);
        config.topology.compile_mode = "flat".to_string();
        let path = write_model(&dir, &config);
        let error = load_model(&path).expect_err("cross-field mode must fail");
        assert!(error.to_string().contains("not supported"), "{error}");
        assert!(
            !Path::new(&topology_source_migration_stamp_path(&path_string(&path))).exists()
        );
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn update_config_normalizes_flags_and_preserves_credentials() {
        let dir = unique_test_dir();
        let mut existing = base_config();
        existing.topology.source = Some(TopologySource::Uisp);
        let path = write_model(&dir, &existing);

        let mut config = base_config();
        config.topology.source = Some(TopologySource::Uisp);
        set_flag(&mut config, TopologySource::Uisp, true);
        set_flag(&mut config, TopologySource::Visp, true);
        config
            .visp_integration
            .as_mut()
            .expect("visp section")
            .client_secret = "keep-me".to_string();
        config.uisp_integration.token = "keep-uisp".to_string();

        let saved = update_config_at(&path_string(&path), &config).expect("save");
        assert_eq!(saved.topology.source, Some(TopologySource::Uisp));
        assert!(saved.uisp_integration.enable_uisp);
        assert!(!saved.visp_integration.as_ref().unwrap().enable_visp);
        assert_eq!(
            saved.visp_integration.as_ref().unwrap().client_secret,
            "keep-me"
        );
        assert_eq!(saved.uisp_integration.token, "keep-uisp");
        assert_eq!(saved.topology_source_runtime.origin, TopologySourceOrigin::Config);
        assert_eq!(stamp_text(&path), Some("settled:uisp\n".to_string()));

        let reloaded = Config::load_from_string(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(reloaded.uisp_integration.enable_uisp);
        assert!(!reloaded.visp_integration.as_ref().unwrap().enable_visp);
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn update_config_file_source_clears_all_flags() {
        let dir = unique_test_dir();
        let mut config = base_config();
        config.topology.source = Some(TopologySource::ShapedDevices);
        set_flag(&mut config, TopologySource::Uisp, true);
        config.uisp_integration.token = "keep-uisp".to_string();
        let path = write_model(&dir, &base_config());

        let saved = update_config_at(&path_string(&path), &config).expect("save");
        assert_eq!(saved.topology.source, Some(TopologySource::ShapedDevices));
        assert!(!saved.uisp_integration.enable_uisp);
        assert!(!saved.splynx_integration.enable_splynx);
        assert!(!saved.powercode_integration.enable_powercode);
        assert!(!saved.sonar_integration.enable_sonar);
        assert!(!saved.netzur_integration.as_ref().unwrap().enable_netzur);
        assert!(!saved.visp_integration.as_ref().unwrap().enable_visp);
        assert!(saved.wispgate_integration.is_none());
        assert_eq!(saved.uisp_integration.token, "keep-uisp");
        assert_eq!(
            stamp_text(&path),
            Some("settled:shaped_devices\n".to_string())
        );
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn update_config_allows_first_run_source_less_default_without_stamp() {
        let dir = unique_test_dir();
        let path = write_model(&dir, &base_config());
        let saved = update_config_at(&path_string(&path), &base_config()).expect("first run");
        assert_eq!(saved.topology.source, None);
        assert_eq!(saved.topology_source_runtime.origin, TopologySourceOrigin::Missing);
        assert!(
            !Path::new(&topology_source_migration_stamp_path(&path_string(&path))).exists()
        );
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn update_config_rejects_missing_source_with_flag_and_bad_mode() {
        let dir = unique_test_dir();
        let path = write_model(&dir, &base_config());

        let mut conflicted = base_config();
        set_flag(&mut conflicted, TopologySource::Uisp, true);
        let error = update_config_at(&path_string(&path), &conflicted)
            .expect_err("missing source + flag must reject");
        assert!(error.to_string().contains("no topology source selected"), "{error}");

        let mut bad_mode = base_config();
        bad_mode.topology.source = Some(TopologySource::Visp);
        bad_mode.topology.compile_mode = "flat".to_string();
        let before = fs::read_to_string(&path).unwrap();
        let error = update_config_at(&path_string(&path), &bad_mode)
            .expect_err("unsupported mode must reject");
        assert!(error.to_string().contains("not supported"), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert!(
            !Path::new(&topology_source_migration_stamp_path(&path_string(&path))).exists()
        );
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn shared_topology_source_predicate_honours_conflict() {
        let mut config = base_config();
        config.topology.source = Some(TopologySource::LibreqosNetwork);
        assert!(topology_source_is(&config, TopologySource::LibreqosNetwork));
        assert!(!topology_source_is(&config, TopologySource::Uisp));

        config.topology_source_runtime.conflict = Some(TopologySourceConflict {
            source: Some(TopologySource::LibreqosNetwork),
            enabled: vec!["enable_uisp".to_string()],
            message: "conflict".to_string(),
        });
        assert!(!topology_source_is(&config, TopologySource::LibreqosNetwork));
    }

}
