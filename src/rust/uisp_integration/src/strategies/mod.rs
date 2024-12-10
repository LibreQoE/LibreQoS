mod flat;
mod full;

use std::sync::Arc;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use lqos_config::Config;
use tracing::{error, info};
use lqos_bus::BlackboardSystem;
use crate::blackboard;

/// Builds the network using the selected strategy.
pub async fn build_with_strategy(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    // Select a Strategy
    blackboard(BlackboardSystem::System, "UISP", config.uisp_integration.strategy.to_lowercase().as_str()).await;
    match config.uisp_integration.strategy.to_lowercase().as_str() {
        "flat" => {
            info!("Strategy selected: flat");
            flat::build_flat_network(config, ip_ranges).await?;
            Ok(())
        }
        "full" => {
            info!("Strategy selected: full");
            full::build_full_network(config, ip_ranges).await?;
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
