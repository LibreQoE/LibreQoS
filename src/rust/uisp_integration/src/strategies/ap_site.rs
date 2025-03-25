use std::collections::HashMap;
use std::fs::write;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};
use lqos_config::Config;
use crate::blackboard_blob;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::full::shaped_devices_writer::ShapedDevice;
use crate::uisp_types::UispSiteType;

/// Creates a network with APs detected from clients,
/// and then a single site above them (shared if the site
/// matches).
pub async fn build_ap_site_network(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {

    Ok(())
}