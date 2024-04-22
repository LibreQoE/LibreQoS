use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use lqos_config::Config;
use tracing::error;

pub async fn build_flat_network(
    _config: Config,
    _ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    error!("Not implemented yet");
    Ok(())
}
