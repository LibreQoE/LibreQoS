use crate::errors::UispIntegrationError;
use lqos_config::Config;
use tracing::error;
use crate::ip_ranges::IpRanges;

pub async fn build_flat_network(_config: Config, _ip_ranges: IpRanges) -> Result<(), UispIntegrationError> {
    error!("Not implemented yet");
    Ok(())
}
