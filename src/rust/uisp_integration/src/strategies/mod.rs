mod ap_only;
mod ap_site;
mod common;
mod flat;
mod full;
mod full2;

use crate::blackboard;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use lqos_bus::BlackboardSystem;
use lqos_config::Config;
use std::sync::Arc;
use tracing::{error, info};

/// Builds the network using the selected strategy.
pub async fn build_with_strategy(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    // Select a Strategy
    blackboard(
        BlackboardSystem::System,
        "UISP",
        config.uisp_integration.strategy.to_lowercase().as_str(),
    )
    .await;
    match config.uisp_integration.strategy.to_lowercase().as_str() {
        "flat" => {
            info!("Strategy selected: flat");
            flat::build_flat_network(config, ip_ranges).await?;
            Ok(())
        }
        /*"full" => {
            info!("Strategy selected: full");
            full::build_full_network(config, ip_ranges).await?;
            Ok(())
        }*/
        "ap_only" => {
            info!("Strategy selected: ap_only");
            ap_only::build_ap_only_network(config, ip_ranges).await?;
            Ok(())
        }
        "ap_site" => {
            info!("Strategy selected: ap_site");
            ap_site::build_ap_site_network(config, ip_ranges).await?;
            Ok(())
        }
        "full2" | "full" => {
            info!("Strategy selected: full2");
            full2::build_full_network_v2(config, ip_ranges).await?;
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
