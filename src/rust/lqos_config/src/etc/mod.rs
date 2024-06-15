//! Manages the `/etc/lqos.conf` file.

mod etclqos_migration;

use std::path::Path;
use self::migration::migrate_if_needed;
pub use self::v15::Config;
pub use etclqos_migration::*;
use std::sync::Mutex;
use thiserror::Error;
mod migration;
mod python_migration;
#[cfg(test)]
pub mod test_data;
mod v15;
pub use v15::{Tunables, BridgeConfig};

static CONFIG: Mutex<Option<Config>> = Mutex::new(None);

/// Load the configuration from `/etc/lqos.conf`.
pub fn load_config() -> Result<Config, LibreQoSConfigError> {
    let mut config_location = "/etc/lqos.conf".to_string();
    if let Ok(lqos_config) = std::env::var("LQOS_CONFIG") {
        config_location = lqos_config;
        log::info!("Overriding lqos.conf location from environment variable.");
    }
    
    let mut lock = CONFIG.lock().unwrap();
    if lock.is_none() {
        log::info!("Loading configuration file {config_location}");
        migrate_if_needed().map_err(|e| {
            log::error!("Unable to migrate configuration: {:?}", e);
            LibreQoSConfigError::FileNotFoud
        })?;

        let file_result = std::fs::read_to_string(&config_location);
        if file_result.is_err() {
            log::error!("Unable to open {config_location}");
            return Err(LibreQoSConfigError::FileNotFoud);
        }
        let raw = file_result.unwrap();

        let config_result = Config::load_from_string(&raw);
        if config_result.is_err() {
            log::error!("Unable to parse /etc/lqos.conf");
            log::error!("Error: {:?}", config_result);
            return Err(LibreQoSConfigError::ParseError(format!(
                "{:?}",
                config_result
            )));
        }
        let mut final_config = config_result.unwrap(); // We know it's good at this point
        
        // Check for environment variable overrides
        if let Ok(lqos_dir) = std::env::var("LQOS_DIRECTORY") {
            final_config.lqos_directory = lqos_dir;
        }
        
        log::info!("Set cached version of config file");
        *lock = Some(final_config);
    }

    Ok(lock.as_ref().unwrap().clone())
}

/// Enables LTS reporting in the configuration file.
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
}

/// Update the configuration on disk
pub fn update_config(new_config: &Config) -> Result<(), LibreQoSConfigError> {
    log::info!("Updating stored configuration");
    let mut lock = CONFIG.lock().unwrap();
    *lock = Some(new_config.clone());

    // Does the configuration exist?
    let config_path = Path::new("/etc/lqos.conf");
    if config_path.exists() {
        let backup_path = Path::new("/etc/lqos.conf.webbackup");
        std::fs::copy(config_path, backup_path)
            .map_err(|e| {
                log::error!("Unable to create backup configuration: {e:?}");
                LibreQoSConfigError::CannotCopy
            })?;
    }
    
    // Serialize the new one
    let serialized = toml::to_string_pretty(new_config)
        .map_err(|e| {
            log::error!("Unable to serialize new configuration to TOML: {e:?}");
            LibreQoSConfigError::SerializeError
        })?;
    std::fs::write(config_path, serialized)
        .map_err(|e| {
            log::error!("Unable to write new configuration: {e:?}");
            LibreQoSConfigError::CannotWrite
        })?;

    Ok(())
}

/// Helper function that disables the XDP bridge in the LIVE, CACHED
/// configuration --- it does NOT save the changes to disk. This is
/// intended for use when the XDP bridge is disabled by pre-flight
/// because of a Linux bridge.
pub fn disable_xdp_bridge() -> Result<(), LibreQoSConfigError> {
    let mut config = load_config()?;
    let mut lock = CONFIG.lock().unwrap();

    if let Some(bridge) = &mut config.bridge {
        bridge.use_xdp_bridge = false;
    }

    // Write the lock
    *lock = Some(config);

    Ok(())
}

#[derive(Debug, Error)]
pub enum LibreQoSConfigError {
    #[error("Unable to read /etc/lqos.conf. See other errors for details.")]
    CannotOpenEtcLqos,
    #[error("Unable to locate (path to LibreQoS)/ispConfig.py. Check your path and that you have configured it.")]
    FileNotFoud,
    #[error("Unable to read the contents of ispConfig.py. Check file permissions.")]
    CannotReadFile,
    #[error("Unable to parse ispConfig.py")]
    ParseError(String),
    #[error("Could not backup configuration")]
    CannotCopy,
    #[error("Could not remove the previous configuration.")]
    CannotRemove,
    #[error("Could not open ispConfig.py for write")]
    CannotOpenForWrite,
    #[error("Unable to write to ispConfig.py")]
    CannotWrite,
    #[error("Unable to read IP")]
    CannotReadIP,
    #[error("Unable to serialize config")]
    SerializeError,
}

