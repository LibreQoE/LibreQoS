use crate::errors::UispIntegrationError;
use lqos_config::Config;
use tracing::error;

pub async fn build_flat_network(_config: Config) -> Result<(), UispIntegrationError> {
    error!("Not implemented yet");
    Ok(())
}
