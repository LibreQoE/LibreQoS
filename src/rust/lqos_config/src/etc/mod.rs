//! Manages the `/etc/lqos.conf` file.

mod etclqos_migration;

use self::migration::migrate_if_needed;
pub use self::v15::Config;
use arc_swap::ArcSwap;
pub use etclqos_migration::*;
use once_cell::sync::Lazy;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tracing::{debug, error, info};

mod migration;
mod python_migration;
#[cfg(test)]
pub mod test_data;
mod v15;
pub use v15::{BridgeConfig, Tunables, LazyQueueMode};

static CONFIG_LOADED: AtomicBool = AtomicBool::new(false);
static CONFIG: Lazy<ArcSwap<Config>> = Lazy::new(|| match load_config() {
    Ok(config) => {
        CONFIG_LOADED.store(true, std::sync::atomic::Ordering::SeqCst);
        ArcSwap::new(config)
    }
    Err(e) => {
        error!("Unable to load configuration: {:?}", e);
        ArcSwap::new(Arc::new(Config::default()))
    }
});

static LOADER_MUTEX: Mutex<bool> = Mutex::new(false);

/// Load the configuration from `/etc/lqos.conf`.
pub fn load_config() -> Result<Arc<Config>, LibreQoSConfigError> {
    // If we have a cached version, return it
    let mut lock = LOADER_MUTEX.lock().unwrap();
    *lock = !(*lock); // Not actually useful, prevents it from being optimized away
    if CONFIG_LOADED.load(std::sync::atomic::Ordering::SeqCst) {
        let clone = CONFIG.load().clone();
        return Ok(clone);
    }

    let config_location = if let Ok(lqos_config) = std::env::var("LQOS_CONFIG") {
        info!("Overriding lqos.conf location from environment variable.");
        lqos_config
    } else {
        "/etc/lqos.conf".to_string()
    };

    debug!("Loading configuration file {config_location}");
    migrate_if_needed(&config_location).map_err(|e| {
        error!("Unable to migrate configuration: {:?}", e);
        LibreQoSConfigError::FileNotFoud
    })?;

    let file_result = std::fs::read_to_string(&config_location);
    if file_result.is_err() {
        error!("Unable to open {config_location}");
        return Err(LibreQoSConfigError::FileNotFoud);
    }
    let raw = file_result.unwrap();

    let config_result = Config::load_from_string(&raw);
    if config_result.is_err() {
        error!("Unable to parse /etc/lqos.conf");
        error!("Error: {:?}", config_result);
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

    debug!("Set cached version of config file");
    let new_config = Arc::new(final_config.clone());

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
    CONFIG.store(Arc::new(new_config.clone()));

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
        LibreQoSConfigError::CannotWrite
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
    CONFIG.store(Arc::new(config));

    Ok(())
}

#[derive(Debug, Error)]
pub enum LibreQoSConfigError {
    #[error("Unable to read /etc/lqos.conf. See other errors for details.")]
    CannotOpenEtcLqos,
    #[error(
        "Unable to locate (path to LibreQoS)/ispConfig.py. Check your path and that you have configured it."
    )]
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
