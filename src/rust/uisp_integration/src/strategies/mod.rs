mod flat;
mod full;

use crate::errors::UispIntegrationError;
use lqos_config::Config;
use tracing::{error, info};

pub async fn build_with_strategy(config: Config) -> Result<(), UispIntegrationError> {
    // Select a Strategy
    match config.uisp_integration.strategy.to_lowercase().as_str() {
        "flat" => {
            info!("Strategy selected: flat");
            flat::build_flat_network(config).await?;
            Ok(())
        }
        "full" => {
            info!("Strategy selected: full");
            full::build_full_network(config).await?;
            Ok(())
        }
        _ => {
            error!(
                "Unknown strategy: {}. Bailing.",
                config.uisp_integration.strategy
            );
            Err(UispIntegrationError::UnknownIntegrationStrategy)
        }
    }
}
