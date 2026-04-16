//! Manages the `/etc/lqos.conf` file.

mod etclqos_migration;

use self::migration::migrate_if_needed;
pub use self::v15::Config;
use arc_swap::ArcSwap;
pub use etclqos_migration::*;
use once_cell::sync::Lazy;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;
use toml_edit::{DocumentMut, Item, Table, value};
use tracing::{debug, error, info};

mod migration;
mod python_migration;
#[cfg(test)]
pub mod test_data;
mod v15;
pub use v15::{
    BridgeConfig, DynamicCircuitRangeRule, DynamicCircuitsConfig, IntegrationConfig, LazyQueueMode,
    MikrotikIpv6Config, QueueMode, RttThresholds, SingleInterfaceConfig, SslConfig,
    StormguardConfig, StormguardStrategy, TopologyConfig, TreeguardCircuitsConfig, TreeguardConfig,
    TreeguardCpuConfig, TreeguardCpuMode, TreeguardLinksConfig, TreeguardQooConfig, Tunables,
    normalize_external_hostname,
};

static CONFIG: Lazy<ArcSwap<Option<Arc<Config>>>> = Lazy::new(|| ArcSwap::from_pointee(None));
static TREEGUARD_CPU_MODE_MIGRATION_NOTICE: Lazy<std::sync::Mutex<Option<String>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

/// Load the configuration from `/etc/lqos.conf`.
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

/// Returns the current TreeGuard CPU-mode migration notice, if an automatic upgrade rewrite
/// occurred during config load.
pub fn treeguard_cpu_mode_migration_notice() -> Option<String> {
    TREEGUARD_CPU_MODE_MIGRATION_NOTICE
        .lock()
        .ok()
        .and_then(|notice| notice.clone())
}

fn treeguard_cpu_mode_migration_stamp_path(config_path: &str) -> String {
    format!("{config_path}.treeguard_cpu_mode_migrated")
}

fn uisp_capacity_defaults_migration_stamp_path(config_path: &str) -> String {
    format!("{config_path}.uisp_capacity_defaults_migrated")
}

fn topology_compile_mode_migration_stamp_path(config_path: &str) -> String {
    format!("{config_path}.topology_compile_mode_migrated")
}

fn treeguard_links_virtualization_migration_stamp_path(config_path: &str) -> String {
    format!("{config_path}.treeguard_links_virtualization_migrated")
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

fn maybe_migrate_treeguard_cpu_mode(
    config_location: &str,
    raw: String,
) -> Result<String, LibreQoSConfigError> {
    let mut doc: DocumentMut = raw.parse().map_err(|e| LibreQoSConfigError::ParseError {
        path: config_location.to_string(),
        details: format!("Error parsing config: {e}"),
    })?;

    let mode_item = doc
        .get("treeguard")
        .and_then(|section| section.as_table_like())
        .and_then(|treeguard| treeguard.get("cpu"))
        .and_then(|cpu| cpu.as_table_like())
        .and_then(|cpu| cpu.get("mode"))
        .and_then(|mode| mode.as_str());
    if mode_item != Some("traffic_rtt_only") {
        return Ok(raw);
    }

    let stamp_path = treeguard_cpu_mode_migration_stamp_path(config_location);
    if Path::new(&stamp_path).exists() {
        return Ok(raw);
    }

    let Some(cpu_table) = doc
        .get_mut("treeguard")
        .and_then(|section| section.as_table_like_mut())
        .and_then(|treeguard| treeguard.get_mut("cpu"))
        .and_then(|cpu| cpu.as_table_like_mut())
    else {
        return Ok(raw);
    };

    cpu_table.insert("mode", value("cpu_aware"));
    let migrated = doc.to_string();

    let config_path = Path::new(config_location);
    if config_path.exists() {
        let backup_path = format!("{config_location}.treeguard_cpu_mode_backup");
        std::fs::copy(config_path, &backup_path).map_err(|e| LibreQoSConfigError::CannotWrite {
            path: backup_path,
            source: e,
        })?;
    }

    std::fs::write(config_path, &migrated).map_err(|e| LibreQoSConfigError::CannotWrite {
        path: config_location.to_string(),
        source: e,
    })?;
    std::fs::write(&stamp_path, b"cpu_aware\n").map_err(|e| LibreQoSConfigError::CannotWrite {
        path: stamp_path,
        source: e,
    })?;

    let notice = "TreeGuard CPU mode was automatically migrated from traffic_rtt_only to cpu_aware during upgrade. CPU-aware mode is now the default and recommended virtualization policy.".to_string();
    info!("{notice}");
    if let Ok(mut slot) = TREEGUARD_CPU_MODE_MIGRATION_NOTICE.lock() {
        *slot = Some(notice);
    }

    Ok(migrated)
}

fn maybe_migrate_treeguard_link_virtualization_defaults(
    config_location: &str,
    raw: String,
) -> Result<String, LibreQoSConfigError> {
    let stamp_path = treeguard_links_virtualization_migration_stamp_path(config_location);
    if Path::new(&stamp_path).exists() {
        return Ok(raw);
    }

    let mut doc: DocumentMut = raw.parse().map_err(|e| LibreQoSConfigError::ParseError {
        path: config_location.to_string(),
        details: format!("Error parsing config: {e}"),
    })?;

    let links_enabled = doc
        .get("treeguard")
        .and_then(|section| section.as_table_like())
        .and_then(|treeguard| treeguard.get("links"))
        .and_then(|links| links.as_table_like())
        .and_then(|links| links.get("enabled"))
        .and_then(|value| value.as_bool());
    let top_level_auto_virtualize = doc
        .get("treeguard")
        .and_then(|section| section.as_table_like())
        .and_then(|treeguard| treeguard.get("links"))
        .and_then(|links| links.as_table_like())
        .and_then(|links| links.get("top_level_auto_virtualize"))
        .and_then(|value| value.as_bool());

    let already_disabled = links_enabled == Some(false) && top_level_auto_virtualize == Some(false);
    if already_disabled {
        std::fs::write(&stamp_path, b"disabled\n").map_err(|e| {
            LibreQoSConfigError::CannotWrite {
                path: stamp_path.clone(),
                source: e,
            }
        })?;
        return Ok(raw);
    }

    if !doc.as_table().contains_key("treeguard") {
        doc["treeguard"] = Item::Table(Table::new());
    }
    if doc["treeguard"].get("links").is_none() {
        doc["treeguard"]["links"] = Item::Table(Table::new());
    }
    doc["treeguard"]["links"]["enabled"] = value(false);
    doc["treeguard"]["links"]["top_level_auto_virtualize"] = value(false);
    let migrated = doc.to_string();

    let config_path = Path::new(config_location);
    if config_path.exists() {
        let backup_path = format!("{config_location}.treeguard_links_virtualization_backup");
        std::fs::copy(config_path, &backup_path).map_err(|e| LibreQoSConfigError::CannotWrite {
            path: backup_path,
            source: e,
        })?;
    }

    std::fs::write(config_path, &migrated).map_err(|e| LibreQoSConfigError::CannotWrite {
        path: config_location.to_string(),
        source: e,
    })?;
    std::fs::write(&stamp_path, b"disabled\n").map_err(|e| LibreQoSConfigError::CannotWrite {
        path: stamp_path,
        source: e,
    })?;

    info!(
        "TreeGuard link virtualization was automatically disabled during upgrade. Static queue policy is now the default baseline."
    );

    Ok(migrated)
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

fn actually_load_from_disk() -> Result<Arc<Config>, LibreQoSConfigError> {
    let config_location = if let Ok(lqos_config) = std::env::var("LQOS_CONFIG") {
        info!("Overriding lqos.conf location from environment variable.");
        lqos_config
    } else {
        "/etc/lqos.conf".to_string()
    };

    debug!("Loading configuration file {config_location}");
    migrate_if_needed(&config_location).map_err(|e| {
        error!("Unable to migrate configuration: {:?}", e);
        match &e {
            migration::MigrationError::ReadError(io)
                if io.kind() == std::io::ErrorKind::NotFound =>
            {
                LibreQoSConfigError::NotFound {
                    path: config_location.clone(),
                }
            }
            _ => LibreQoSConfigError::MigrationFailed {
                path: config_location.clone(),
                details: e.to_string(),
            },
        }
    })?;

    let raw = std::fs::read_to_string(&config_location).map_err(|e| {
        error!("Unable to read {config_location}: {e:?}");
        if e.kind() == std::io::ErrorKind::NotFound {
            LibreQoSConfigError::NotFound {
                path: config_location.clone(),
            }
        } else {
            LibreQoSConfigError::CannotRead {
                path: config_location.clone(),
                source: e,
            }
        }
    })?;

    let raw = maybe_migrate_treeguard_cpu_mode(&config_location, raw)?;
    let raw = maybe_migrate_treeguard_link_virtualization_defaults(&config_location, raw)?;
    let raw = maybe_migrate_uisp_capacity_defaults(&config_location, raw)?;
    let raw = maybe_migrate_topology_compile_mode(&config_location, raw)?;

    let mut final_config = Config::load_from_string(&raw).map_err(|e| {
        error!("Unable to parse {config_location}");
        LibreQoSConfigError::ParseError {
            path: config_location.clone(),
            details: e,
        }
    })?;

    // Check for environment variable overrides
    if let Ok(lqos_dir) = std::env::var("LQOS_DIRECTORY") {
        final_config.lqos_directory = lqos_dir;
    }
    crate::runtime_state_migration::migrate_legacy_runtime_state(&final_config).map_err(|e| {
        error!("Unable to migrate legacy runtime state: {e:?}");
        LibreQoSConfigError::MigrationFailed {
            path: final_config.lqos_directory.clone(),
            details: e.to_string(),
        }
    })?;
    crate::migrate_legacy_mikrotik_ipv6_credentials(&final_config).map_err(|e| {
        error!("Unable to migrate legacy Mikrotik IPv6 credentials: {e:?}");
        LibreQoSConfigError::MigrationFailed {
            path: final_config
                .resolved_mikrotik_ipv6_config_path()
                .display()
                .to_string(),
            details: e.to_string(),
        }
    })?;

    debug!("Set cached version of config file");
    let new_config = Arc::new(final_config);

    Ok(new_config)
}

/*/// Enables LTS reporting in the configuration file.
pub fn enable_long_term_stats(license_key: String) -> Result<(), LibreQoSConfigError> {
    let mut config = load_config()?;
    let mut lock = CONFIG.lock().unwrap();

    config.long_term_stats.gather_stats = true;
    config.long_term_stats.collation_period_seconds = 60;
    config.long_term_stats.license_key = Some(license_key);
    if config.uisp_integration.enable_uisp {
        config.long_term_stats.uisp_reporting_interval_seconds = Some(300);
    }

    // Write the file
    let raw = toml::to_string_pretty(&config).unwrap();
    std::fs::write("/etc/lqos.conf", raw).map_err(|_| LibreQoSConfigError::CannotWrite)?;

    // Write the lock
    *lock = Some(config);

    Ok(())
}*/

/// Update the configuration on disk
pub fn update_config(new_config: &Config) -> Result<(), LibreQoSConfigError> {
    debug!("Updating stored configuration");
    CONFIG.store(Some(Arc::new(new_config.clone())).into());

    // Does the configuration exist?
    let config_path = Path::new("/etc/lqos.conf");
    if config_path.exists() {
        let backup_path = Path::new("/etc/lqos.conf.webbackup");
        std::fs::copy(config_path, backup_path).map_err(|e| {
            error!("Unable to create backup configuration: {e:?}");
            LibreQoSConfigError::CannotCopy
        })?;
    }

    // Serialize the new one
    let serialized = toml::to_string_pretty(new_config).map_err(|e| {
        error!("Unable to serialize new configuration to TOML: {e:?}");
        LibreQoSConfigError::SerializeError
    })?;
    std::fs::write(config_path, serialized).map_err(|e| {
        error!("Unable to write new configuration: {e:?}");
        LibreQoSConfigError::CannotWrite {
            path: config_path.display().to_string(),
            source: e,
        }
    })?;

    Ok(())
}

/// Helper function that disables the XDP bridge in the LIVE, CACHED
/// configuration --- it does NOT save the changes to disk. This is
/// intended for use when the XDP bridge is disabled by pre-flight
/// because of a Linux bridge.
pub fn disable_xdp_bridge() -> Result<(), LibreQoSConfigError> {
    let config = load_config()?;
    let mut config = (*config).clone();

    if let Some(bridge) = &mut config.bridge {
        bridge.use_xdp_bridge = false;
    }

    // Write the lock
    CONFIG.store(Some(Arc::new(config)).into());

    Ok(())
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
        maybe_migrate_topology_compile_mode, maybe_migrate_treeguard_link_virtualization_defaults,
        maybe_migrate_uisp_capacity_defaults, topology_compile_mode_migration_stamp_path,
        treeguard_links_virtualization_migration_stamp_path,
        uisp_capacity_defaults_migration_stamp_path,
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

    #[test]
    fn treeguard_link_virtualization_migration_disables_runtime_link_virtualization() {
        let test_dir = unique_test_dir();
        let raw = "[treeguard]\nenabled = true\n\n[treeguard.links]\nenabled = true\ntop_level_auto_virtualize = true\n";
        let config_path = write_test_config(&test_dir, raw);

        let config_path_str = path_string(&config_path);
        let migrated =
            maybe_migrate_treeguard_link_virtualization_defaults(&config_path_str, raw.to_string())
                .expect("migration should succeed");

        assert!(migrated.contains("[treeguard.links]"));
        assert!(migrated.contains("enabled = false"));
        assert!(migrated.contains("top_level_auto_virtualize = false"));
        assert!(
            Path::new(&treeguard_links_virtualization_migration_stamp_path(
                &config_path_str
            ))
            .exists()
        );

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }

    #[test]
    fn treeguard_link_virtualization_migration_stamps_when_already_disabled() {
        let test_dir = unique_test_dir();
        let raw = "[treeguard]\nenabled = true\n\n[treeguard.links]\nenabled = false\ntop_level_auto_virtualize = false\n";
        let config_path = write_test_config(&test_dir, raw);

        let config_path_str = path_string(&config_path);
        let migrated =
            maybe_migrate_treeguard_link_virtualization_defaults(&config_path_str, raw.to_string())
                .expect("migration should succeed");

        assert_eq!(migrated, raw);
        assert_eq!(
            fs::read_to_string(&config_path).expect("config should remain unchanged"),
            raw
        );
        assert!(
            Path::new(&treeguard_links_virtualization_migration_stamp_path(
                &config_path_str
            ))
            .exists()
        );

        fs::remove_dir_all(&test_dir).expect("should clean up temp dir");
    }
}
