use super::{
    EtcLqos, EtcLqosError,
    python_migration::{PythonMigration, PythonMigrationError},
    v15::{BridgeConfig, Config, SingleInterfaceConfig},
};
use crate::etc::v15::influxdb::InfluxDbConfig;
/// Provides support for migration from older versions of the configuration file.
use std::path::Path;
use thiserror::Error;
use toml_edit::DocumentMut;
use tracing::{debug, error, info};

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("Invalid Config File Version")]
    InvalidVersion,
    #[error("Failed to read configuration file: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("Failed to parse configuration file: {0}")]
    ParseError(#[from] toml_edit::TomlError),
    #[error("Unknown Version: {0}")]
    UnknownVersion(String),
    #[error("Unable to load old version: {0}")]
    LoadError(#[from] EtcLqosError),
    #[error("Unable to load python version: {0}")]
    PythonLoadError(#[from] PythonMigrationError),
    #[error("Unable to serialize TOML")]
    SerializeError,
    #[error("Should never happen")]
    ImpossibleError,
}

pub fn migrate_if_needed(config_location: &str) -> Result<(), MigrationError> {
    debug!("Checking config file version");
    let raw = std::fs::read_to_string(config_location).map_err(MigrationError::ReadError)?;

    let doc = raw
        .parse::<DocumentMut>()
        .map_err(MigrationError::ParseError)?;
    if let Some((_key, version)) = doc.get_key_value("version") {
        debug!(
            "Configuration file is at version {}",
            version.as_str().ok_or(MigrationError::InvalidVersion)?
        );
        if version
            .as_str()
            .ok_or(MigrationError::InvalidVersion)?
            .trim()
            == "1.5"
        {
            debug!("Configuration file is already at version 1.5, no migration needed");
            return Ok(());
        } else {
            error!(
                "Configuration file is at version {}, but this version of lqos only supports version 1.5",
                version.as_str().ok_or(MigrationError::InvalidVersion)?
            );
            return Err(MigrationError::UnknownVersion(
                version
                    .as_str()
                    .ok_or(MigrationError::InvalidVersion)?
                    .to_string(),
            ));
        }
    } else {
        // If the file otherwise looks like a v1.5 config but is missing the `version` key,
        // add it in-place. This avoids triggering 1.4->1.5 migration (which requires
        // legacy `ispConfig.py`) for modern installs with a versionless config.
        info!("No version found in configuration file; attempting to treat it as version 1.5 without an explicit version key");
        let mut with_version = doc.clone();
        with_version.insert("version", toml_edit::value("1.5"));
        if Config::load_from_string(&with_version.to_string()).is_ok() {
            let backup_path = format!("{config_location}.backup_noversion");
            std::fs::copy(config_location, &backup_path).map_err(MigrationError::ReadError)?;
            std::fs::write(config_location, with_version.to_string()).map_err(MigrationError::ReadError)?;
            info!("Added missing `version = \"1.5\"` to {config_location}; backup written to {backup_path}");
            return Ok(());
        }

        info!("No version found in configuration file, assuming 1.4x and migration is needed");
        let new_config = migrate_14_to_15()?;
        // Back up the old configuration
        let backup_path = format!("{config_location}.backup14");
        std::fs::rename(config_location, &backup_path).map_err(MigrationError::ReadError)?;

        // Rename the old Python configuration
        let from = Path::new(new_config.lqos_directory.as_str()).join("ispConfig.py");
        let to = Path::new(new_config.lqos_directory.as_str()).join("ispConfig.py.backup14");

        std::fs::rename(from, to).map_err(MigrationError::ReadError)?;

        // Save the configuration
        let raw =
            toml::to_string_pretty(&new_config).map_err(|_| MigrationError::SerializeError)?;
        std::fs::write(config_location, raw).map_err(MigrationError::ReadError)?;
    }

    Ok(())
}

fn migrate_14_to_15() -> Result<Config, MigrationError> {
    // Load the 1.4 config file
    let old_config = EtcLqos::load().map_err(MigrationError::LoadError)?;
    let python_config = PythonMigration::load().map_err(MigrationError::PythonLoadError)?;
    let new_config = do_migration_14_to_15(&old_config, &python_config)?;
    Ok(new_config)
}

fn do_migration_14_to_15(
    old_config: &EtcLqos,
    python_config: &PythonMigration,
) -> Result<Config, MigrationError> {
    // This is separated out to make unit testing easier
    let mut new_config = Config::default();

    migrate_top_level(old_config, &mut new_config)?;
    migrate_tunables(old_config, &mut new_config)?;
    migrate_bridge(old_config, python_config, &mut new_config)?;
    migrate_lts(old_config, &mut new_config)?;
    migrate_ip_ranges(python_config, &mut new_config)?;
    migrate_integration_common(python_config, &mut new_config)?;
    migrate_splynx(python_config, &mut new_config)?;
    migrate_uisp(python_config, &mut new_config)?;
    migrate_powercode(python_config, &mut new_config)?;
    migrate_sonar(python_config, &mut new_config)?;
    migrate_queues(python_config, &mut new_config)?;
    migrate_influx(python_config, &mut new_config)?;

    new_config
        .validate()
        .map_err(|_| MigrationError::ImpossibleError)?; // Left as an upwrap because this should *never* happen
    Ok(new_config)
}

fn migrate_top_level(old_config: &EtcLqos, new_config: &mut Config) -> Result<(), MigrationError> {
    new_config.version = "1.5".to_string();
    new_config.lqos_directory = old_config.lqos_directory.clone();
    new_config.packet_capture_time = old_config.packet_capture_time.unwrap_or(10);
    if let Some(node_id) = &old_config.node_id {
        new_config.node_id = node_id.clone();
    } else {
        new_config.node_id = Config::calculate_node_id();
    }
    if let Some(node_name) = &old_config.node_name {
        new_config.node_name = node_name.clone();
    } else {
        new_config.node_name = "Set my name in /etc/lqos.conf".to_string();
    }
    Ok(())
}

fn migrate_tunables(old_config: &EtcLqos, new_config: &mut Config) -> Result<(), MigrationError> {
    if let Some(tunables) = &old_config.tuning {
        new_config.tuning.stop_irq_balance = tunables.stop_irq_balance;
        new_config.tuning.netdev_budget_packets = tunables.netdev_budget_packets;
        new_config.tuning.netdev_budget_usecs = tunables.netdev_budget_usecs;
        new_config.tuning.rx_usecs = tunables.rx_usecs;
        new_config.tuning.tx_usecs = tunables.tx_usecs;
        new_config.tuning.disable_txvlan = tunables.disable_txvlan;
        new_config.tuning.disable_rxvlan = tunables.disable_rxvlan;
        new_config.tuning.disable_offload = tunables.disable_offload.clone();
    } else {
        new_config.tuning = Default::default();
    }
    Ok(())
}

fn migrate_bridge(
    old_config: &EtcLqos,
    python_config: &PythonMigration,
    new_config: &mut Config,
) -> Result<(), MigrationError> {
    if python_config.on_astick {
        new_config.bridge = None;
        new_config.single_interface = Some(SingleInterfaceConfig {
            interface: python_config.interface_a.clone(),
            internet_vlan: python_config.stick_vlan_a as u32,
            network_vlan: python_config.stick_vlan_b as u32,
        });
    } else {
        new_config.single_interface = None;
        new_config.bridge = Some(BridgeConfig {
            use_xdp_bridge: old_config
                .bridge
                .as_ref()
                .ok_or(MigrationError::SerializeError)?
                .use_xdp_bridge,
            to_internet: python_config.interface_b.clone(),
            to_network: python_config.interface_a.clone(),
        });
    }
    Ok(())
}

fn migrate_queues(
    python_config: &PythonMigration,
    new_config: &mut Config,
) -> Result<(), MigrationError> {
    new_config.queues.default_sqm = python_config.sqm.clone();
    new_config.queues.monitor_only = python_config.monitor_only_mode;
    new_config.queues.uplink_bandwidth_mbps =
        python_config.upstream_bandwidth_capacity_upload_mbps as u64;
    new_config.queues.downlink_bandwidth_mbps =
        python_config.upstream_bandwidth_capacity_download_mbps as u64;
    new_config.queues.generated_pn_upload_mbps = python_config.generated_pnupload_mbps as u64;
    new_config.queues.generated_pn_download_mbps = python_config.generated_pndownload_mbps as u64;
    new_config.queues.dry_run = !python_config.enable_actual_shell_commands;
    new_config.queues.sudo = python_config.run_shell_commands_as_sudo;
    if python_config.queues_available_override == 0 {
        new_config.queues.override_available_queues = None;
    } else {
        new_config.queues.override_available_queues =
            Some(python_config.queues_available_override as u32);
    }
    new_config.queues.use_binpacking = python_config.use_bin_packing_to_balance_cpu;
    Ok(())
}

fn migrate_lts(old_config: &EtcLqos, new_config: &mut Config) -> Result<(), MigrationError> {
    if let Some(lts) = &old_config.long_term_stats {
        new_config.long_term_stats.gather_stats = lts.gather_stats;
        new_config.long_term_stats.collation_period_seconds = lts.collation_period_seconds;
        new_config.long_term_stats.license_key = lts.license_key.clone();
        new_config.long_term_stats.uisp_reporting_interval_seconds =
            lts.uisp_reporting_interval_seconds;
    } else {
        new_config.long_term_stats = super::v15::LongTermStats::default();
    }
    Ok(())
}

fn migrate_ip_ranges(
    python_config: &PythonMigration,
    new_config: &mut Config,
) -> Result<(), MigrationError> {
    new_config.ip_ranges.ignore_subnets = python_config.ignore_subnets.clone();
    new_config.ip_ranges.allow_subnets = python_config.allowed_subnets.clone();
    Ok(())
}

fn migrate_integration_common(
    python_config: &PythonMigration,
    new_config: &mut Config,
) -> Result<(), MigrationError> {
    new_config.integration_common.circuit_name_as_address = python_config.circuit_name_use_address;
    new_config.integration_common.always_overwrite_network_json =
        python_config.overwrite_network_jsonalways;
    new_config.integration_common.queue_refresh_interval_mins =
        python_config.queue_refresh_interval_mins as u32;
    new_config.integration_common.use_mikrotik_ipv6 = python_config.find_ipv6using_mikrotik_api;
    Ok(())
}

fn migrate_splynx(
    python_config: &PythonMigration,
    new_config: &mut Config,
) -> Result<(), MigrationError> {
    new_config.splynx_integration.enable_splynx = python_config.automatic_import_splynx;
    new_config.splynx_integration.api_key = python_config.splynx_api_key.clone();
    new_config.splynx_integration.api_secret = python_config.splynx_api_secret.clone();
    new_config.splynx_integration.url = python_config.splynx_api_url.clone();
    Ok(())
}

fn migrate_powercode(
    python_config: &PythonMigration,
    new_config: &mut Config,
) -> Result<(), MigrationError> {
    new_config.powercode_integration.enable_powercode = python_config.automatic_import_powercode;
    new_config.powercode_integration.powercode_api_url = python_config.powercode_api_url.clone();
    new_config.powercode_integration.powercode_api_key = python_config.powercode_api_key.clone();
    Ok(())
}

fn migrate_sonar(
    python_config: &PythonMigration,
    new_config: &mut Config,
) -> Result<(), MigrationError> {
    new_config.sonar_integration.enable_sonar = python_config.automatic_import_sonar;
    new_config.sonar_integration.sonar_api_url = python_config.sonar_api_url.clone();
    new_config.sonar_integration.sonar_api_key = python_config.sonar_api_key.clone();
    new_config.sonar_integration.snmp_community = python_config.snmp_community.clone();
    Ok(())
}

fn migrate_uisp(
    python_config: &PythonMigration,
    new_config: &mut Config,
) -> Result<(), MigrationError> {
    new_config.uisp_integration.enable_uisp = python_config.automatic_import_uisp;
    new_config.uisp_integration.token = python_config.uisp_auth_token.clone();
    new_config.uisp_integration.url = python_config.uispbase_url.clone();
    new_config.uisp_integration.site = python_config.uisp_site.clone();
    new_config.uisp_integration.strategy = python_config.uisp_strategy.clone();
    new_config.uisp_integration.suspended_strategy = python_config.uisp_suspended_strategy.clone();
    new_config.uisp_integration.airmax_capacity = python_config.air_max_capacity as f32;
    new_config.uisp_integration.ltu_capacity = python_config.ltu_capacity as f32;
    new_config.uisp_integration.exclude_sites = python_config.exclude_sites.clone();
    new_config.uisp_integration.ipv6_with_mikrotik = python_config.find_ipv6using_mikrotik_api;
    new_config.uisp_integration.bandwidth_overhead_factor =
        python_config.bandwidth_overhead_factor as f32;
    new_config.uisp_integration.commit_bandwidth_multiplier =
        python_config.committed_bandwidth_multiplier as f32;
    // TODO: ExceptionCPEs is going to require some real work
    Ok(())
}

fn migrate_influx(
    python_config: &PythonMigration,
    new_config: &mut Config,
) -> Result<(), MigrationError> {
    if python_config.influx_enabled {
        let cfg = InfluxDbConfig {
            enable_influxdb: python_config.influx_enabled,
            url: python_config.influx_dburl.clone(),
            bucket: python_config.influx_dbbucket.clone(),
            org: python_config.influx_dborg.clone(),
            token: python_config.influx_dbtoken.clone(),
        };
        new_config.influxdb = Some(cfg);
    }
    Ok(())
}
