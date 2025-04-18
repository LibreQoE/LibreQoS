use std::collections::HashMap;
use tracing::{debug, error, info};
use crate::queue_structure::find_queue_bandwidth;

pub struct WatchingSite {
    pub name: String,
    pub max_download_mbps: u64,
    pub max_upload_mbps: u64,
    pub min_download_mbps: u64,
    pub min_upload_mbps: u64,
    pub step_download_mbps: u64,
    pub step_upload_mbps: u64,
}

pub struct TornadoConfig {
    pub sites: HashMap<String, WatchingSite>,
    pub download_interface: String,
    pub upload_interface: String,
    pub dry_run: bool,
    pub log_filename: Option<String>,
}

pub fn configure() -> anyhow::Result<TornadoConfig> {
    debug!("Configuring LibreQoS Tornado...");
    let config = lqos_config::load_config()?;

    if config.on_a_stick_mode() {
        error!("LibreQoS Tornado is not supported in 'on-a-stick' mode.");
        return Err(anyhow::anyhow!("LibreQoS Tornado is not supported in 'on-a-stick' mode."));
    }

    let Some(tornado_config) = &config.tornado else {
        error!("Tornado is not enabled in the configuration.");
        return Err(anyhow::anyhow!("Tornado is not enabled in the configuration."));
    };
    if !tornado_config.enabled {
        error!("Tornado is not enabled in the configuration.");
        return Err(anyhow::anyhow!("Tornado is not enabled in the configuration."));
    }

    let mut sites = HashMap::new();
    for target in &tornado_config.targets {
        let _ = find_queue_bandwidth(&target.name).inspect_err(|e| {
            error!("Error finding queue bandwidth for {}: {:?}", target.name, e);
        })?;
        let site = WatchingSite {
            name: target.name.to_owned(),
            max_download_mbps: target.max_mbps[0],
            max_upload_mbps: target.max_mbps[1],
            min_download_mbps: target.min_mbps[0],
            min_upload_mbps: target.min_mbps[1],
            step_download_mbps: target.step_mbps[0],
            step_upload_mbps: target.step_mbps[1],
        };
        sites.insert(target.name.to_owned(), site);
    }

    let result = TornadoConfig {
        sites,
        download_interface: config.isp_interface().clone(),
        upload_interface: config.internet_interface().clone(),
        dry_run: tornado_config.dry_run,
        log_filename: tornado_config.log_file.clone(),
    };

    Ok(result)
}