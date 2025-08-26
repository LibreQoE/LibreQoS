use std::collections::HashMap;
use allocative::Allocative;
use tracing::{debug, info};
use lqos_bus::TcHandle;
use crate::queue_structure::{find_queue_bandwidth, find_queue_dependents};
use crate::STORMGUARD_STATS;

#[derive(Allocative, Clone)]
pub struct WatchingSite {
    pub name: String,
    pub max_download_mbps: u64,
    pub max_upload_mbps: u64,
    pub min_download_mbps: u64,
    pub min_upload_mbps: u64,
    pub dependent_nodes: Vec<WatchingSiteDependency>,
}

#[derive(Allocative, Clone)]
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

impl StormguardConfig {
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    pub fn refresh_sites(&mut self) {
        let Ok(config) = lqos_config::load_config() else {
            debug!("Unable to reload configuration to refresh StormGuard sites.");
            return;
        };
        let Some(sg_config) = &config.stormguard else {
            debug!("StormGuard is not enabled in the configuration.");
            return;
        };

        // Clear existing stats
        {
            let mut lock = STORMGUARD_STATS.lock();
            lock.clear();
        }
        let new_sites = get_sites_from_queueing_structure(sg_config);
        self.sites = new_sites;
    }
}

pub fn configure() -> anyhow::Result<StormguardConfig> {
    debug!("Configuring LibreQoS StormGuard...");
    let config = lqos_config::load_config()?;

    if config.on_a_stick_mode() {
        info!("LibreQoS StormGuard is not supported in 'on-a-stick' mode.");
        return Err(anyhow::anyhow!("LibreQoS StormGuard is not supported in 'on-a-stick' mode."));
    }

    let Some(sg_config) = &config.stormguard else {
        debug!("StormGuard is not enabled in the configuration.");
        return Err(anyhow::anyhow!("StormGuard is not enabled in the configuration."));
    };
    if !sg_config.enabled {
        debug!("StormGuard is not enabled in the configuration.");
        return Err(anyhow::anyhow!("StormGuard is not enabled in the configuration."));
    }

    let sites = get_sites_from_queueing_structure(&sg_config);

    let result = StormguardConfig {
        sites,
        download_interface: config.isp_interface().clone(),
        upload_interface: config.internet_interface().clone(),
        dry_run: sg_config.dry_run,
        log_filename: sg_config.log_file.clone(),
    };

    Ok(result)
}

fn get_sites_from_queueing_structure(sg_config: &lqos_config::StormguardConfig) -> HashMap<String, WatchingSite> {
    let mut sites = HashMap::new();
    for target in &sg_config.targets {
        let Ok((max_down, max_up)) = find_queue_bandwidth(&target) else {
            debug!("Error finding queue bandwidth for {}", target);
            continue;
        };
        let Ok(dependencies) = find_queue_dependents(&target) else {
            debug!("Error finding queue dependencies for {}", target);
            continue;
        };
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
        {
            let mut lock = STORMGUARD_STATS.lock();
            lock.push((target.to_owned(), max_down, max_up));
        }
    }
    sites
    }