use std::collections::HashMap;
use tracing::{debug, error};
use lqos_bus::TcHandle;
use crate::queue_structure::{find_queue_bandwidth, find_queue_dependents};

pub struct WatchingSite {
    pub name: String,
    pub max_download_mbps: u64,
    pub max_upload_mbps: u64,
    pub min_download_mbps: u64,
    pub min_upload_mbps: u64,
    pub dependent_nodes: Vec<WatchingSiteDependency>,
}

pub struct WatchingSiteDependency {
    pub name: String,
    pub class_id: TcHandle,
    pub original_max_download_mbps: u64,
    pub original_max_upload_mbps: u64,
}

pub struct StormguardConfig {
    pub sites: HashMap<String, WatchingSite>,
    pub download_interface: String,
    pub upload_interface: String,
    pub dry_run: bool,
    pub log_filename: Option<String>,
}

pub fn configure() -> anyhow::Result<StormguardConfig> {
    debug!("Configuring LibreQoS StormGuard...");
    let config = lqos_config::load_config()?;

    if config.on_a_stick_mode() {
        error!("LibreQoS StormGuard is not supported in 'on-a-stick' mode.");
        return Err(anyhow::anyhow!("LibreQoS StormGuard is not supported in 'on-a-stick' mode."));
    }

    let Some(sg_config) = &config.stormguard else {
        tracing::info!("StormGuard is not enabled in the configuration.");
        return Err(anyhow::anyhow!("StormGuard is not enabled in the configuration."));
    };
    if !sg_config.enabled {
        tracing::info!("StormGuard is not enabled in the configuration.");
        return Err(anyhow::anyhow!("StormGuard is not enabled in the configuration."));
    }

    let mut sites = HashMap::new();
    for target in &sg_config.targets {
        let (max_down, max_up) = find_queue_bandwidth(&target).inspect_err(|e| {
            error!("Error finding queue bandwidth for {}: {:?}", target, e);
        })?;
        let dependencies = find_queue_dependents(&target).inspect_err(|e| {
            error!("Error finding queue dependencies for {}: {:?}", target, e);
        })?;
        let min_down = (max_down as f32 * sg_config.minimum_download_percentage) as u64;
        let min_up = (max_up as f32 * sg_config.minimum_upload_percentage) as u64;
        let site = WatchingSite {
            name: target.to_owned(),
            max_download_mbps: max_down,
            max_upload_mbps: max_up,
            min_download_mbps: min_down,
            min_upload_mbps: min_up,
            dependent_nodes: dependencies,
        };
        sites.insert(target.to_owned(), site);
    }

    let result = StormguardConfig {
        sites,
        download_interface: config.isp_interface().clone(),
        upload_interface: config.internet_interface().clone(),
        dry_run: sg_config.dry_run,
        log_filename: sg_config.log_file.clone(),
    };

    Ok(result)
}