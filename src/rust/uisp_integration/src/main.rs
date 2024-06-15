//! Rust version of the UISP Integration from LibreQoS. This will probably
//! be ported back to Python, with Rust support structures - but I'll iterate
//! faster in Rust.

#[warn(missing_docs)]

mod errors;
pub mod ip_ranges;
mod strategies;
pub mod uisp_types;

use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use lqos_config::Config;
use tokio::time::Instant;
use tracing::{error, info};

/// Start the tracing/logging system
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_file(true)
        .with_line_number(true)
        .compact()
        .init();
}

fn check_enabled_status(config: &Config) -> Result<(), UispIntegrationError> {
    if !config.uisp_integration.enable_uisp {
        error!("UISP Integration is disabled in /etc/lqos.conf");
        error!("Integration will not run.");
        Err(UispIntegrationError::IntegrationDisabled)
    } else {
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), UispIntegrationError> {
    let now = Instant::now();
    init_tracing();
    info!("UISP Integration 2.0-rust");

    // Load the configuration
    info!("Loading Configuration");
    let config = lqos_config::load_config().map_err(|e| {
        error!("Unable to load configuration");
        error!("{e:?}");
        UispIntegrationError::CannotLoadConfig
    })?;

    // Check that we're allowed to run
    check_enabled_status(&config)?;

    // Build our allowed/excluded IP ranges
    let ip_ranges = IpRanges::new(&config)?;

    // Select a strategy and go from there
    strategies::build_with_strategy(config, ip_ranges).await?;

    // Print timings
    let elapsed = now.elapsed();
    info!(
        "UISP Integration Run Completed in {:.3} seconds",
        elapsed.as_secs_f32()
    );

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use lqos_config::Config;

    #[test]
    fn test_uisp_disabled() {
        let mut cfg = Config::default();
        cfg.uisp_integration.enable_uisp = false;
        let result = check_enabled_status(&cfg);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            UispIntegrationError::IntegrationDisabled
        );
    }

    #[test]
    fn test_uisp_enabled() {
        let mut cfg = Config::default();
        cfg.uisp_integration.enable_uisp = true;
        let result = check_enabled_status(&cfg);
        assert!(result.is_ok());
    }
}
