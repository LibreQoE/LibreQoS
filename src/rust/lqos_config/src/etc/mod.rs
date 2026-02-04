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
use tracing::{debug, error, info};

mod migration;
mod python_migration;
#[cfg(test)]
pub mod test_data;
mod v15;
pub use v15::{BridgeConfig, LazyQueueMode, SingleInterfaceConfig, StormguardConfig, Tunables};

static CONFIG: Lazy<ArcSwap<Option<Arc<Config>>>> = Lazy::new(|| ArcSwap::from_pointee(None));

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
    #[error("Unable to locate LibreQoS configuration at {path}. Set `LQOS_CONFIG` to override the path.")]
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
