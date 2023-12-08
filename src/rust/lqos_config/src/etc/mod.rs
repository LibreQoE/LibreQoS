//! Manages the `/etc/lqos.conf` file.

mod etclqos_migration;
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
pub use v15::Tunables;

static CONFIG: Mutex<Option<Config>> = Mutex::new(None);

/// Load the configuration from `/etc/lqos.conf`.
pub fn load_config() -> Result<Config, LibreQoSConfigError> {
    log::info!("Loading configuration file /etc/lqos.conf");
    let mut lock = CONFIG.lock().unwrap();
    if lock.is_none() {
        migrate_if_needed().map_err(|e| {
            log::error!("Unable to migrate configuration: {:?}", e);
            LibreQoSConfigError::FileNotFoud
        })?;

        let file_result = std::fs::read_to_string("/etc/lqos.conf");
        if file_result.is_err() {
            log::error!("Unable to open /etc/lqos.conf");
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
        *lock = Some(config_result.unwrap());
    }

    Ok(lock.as_ref().unwrap().clone())
}

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
}
