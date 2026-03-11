use crate::STORMGUARD_STATS;
use crate::queue_structure::{
    all_candidate_site_names, find_queue_bandwidth, find_queue_dependents,
};
use allocative::Allocative;
use lqos_bus::TcHandle;
use lqos_overrides::{NetworkAdjustment, OverrideLayer, OverrideStore};
use std::collections::{HashMap, HashSet};
use tracing::{debug, info, warn};

use lqos_config::StormguardStrategy;

#[derive(Allocative, Clone)]
pub struct WatchingSite {
    pub name: String,
    pub max_download_mbps: u64,
    pub max_upload_mbps: u64,
    pub min_download_mbps: u64,
    pub min_upload_mbps: u64,
    pub dependent_nodes: Vec<WatchingSiteDependency>,
    pub current_download_mbps: u64,
    pub current_upload_mbps: u64,
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
    pub strategy: StormguardStrategy,
    pub increase_fast_multiplier: f64,
    pub increase_multiplier: f64,
    pub decrease_multiplier: f64,
    pub decrease_fast_multiplier: f64,
    pub increase_fast_cooldown_seconds: f32,
    pub increase_cooldown_seconds: f32,
    pub decrease_cooldown_seconds: f32,
    pub decrease_fast_cooldown_seconds: f32,
    pub circuit_fallback_enabled: bool,
    pub circuit_fallback_persist: bool,
    pub circuit_fallback_sqm: String,
    pub delay_threshold_ms: f32,
    pub delay_threshold_ratio: f32,
    pub baseline_alpha_up: f32,
    pub baseline_alpha_down: f32,
    pub probe_interval_seconds: f32,
    pub min_throughput_mbps_for_rtt: f32,
    pub active_ping_target: String,
    pub active_ping_interval_seconds: f32,
    pub active_ping_weight: f32,
    pub active_ping_timeout_seconds: f32,
}

impl StormguardConfig {
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }
}

pub fn configure() -> anyhow::Result<StormguardConfig> {
    debug!("Configuring LibreQoS StormGuard...");
    let config = lqos_config::load_config()?;

    if config.on_a_stick_mode() {
        info!("LibreQoS StormGuard is not supported in 'on-a-stick' mode.");
        return Err(anyhow::anyhow!(
            "LibreQoS StormGuard is not supported in 'on-a-stick' mode."
        ));
    }

    let Some(sg_config) = &config.stormguard else {
        debug!("StormGuard is not enabled in the configuration.");
        return Err(anyhow::anyhow!(
            "StormGuard is not enabled in the configuration."
        ));
    };
    if !sg_config.enabled {
        debug!("StormGuard is not enabled in the configuration.");
        return Err(anyhow::anyhow!(
            "StormGuard is not enabled in the configuration."
        ));
    }

    let persisted_site_overrides = if sg_config.dry_run {
        HashMap::new()
    } else {
        load_stormguard_site_overrides()
    };
    let sites = get_sites_from_queueing_structure(sg_config, &persisted_site_overrides);

    let result = StormguardConfig {
        sites,
        download_interface: config.isp_interface().clone(),
        upload_interface: config.internet_interface().clone(),
        dry_run: sg_config.dry_run,
        log_filename: sg_config.log_file.clone(),
        strategy: sg_config.strategy,
        increase_fast_multiplier: sg_config.increase_fast_multiplier as f64,
        increase_multiplier: sg_config.increase_multiplier as f64,
        decrease_multiplier: sg_config.decrease_multiplier as f64,
        decrease_fast_multiplier: sg_config.decrease_fast_multiplier as f64,
        increase_fast_cooldown_seconds: sg_config.increase_fast_cooldown_seconds,
        increase_cooldown_seconds: sg_config.increase_cooldown_seconds,
        decrease_cooldown_seconds: sg_config.decrease_cooldown_seconds,
        decrease_fast_cooldown_seconds: sg_config.decrease_fast_cooldown_seconds,
        circuit_fallback_enabled: sg_config.circuit_fallback_enabled,
        circuit_fallback_persist: sg_config.circuit_fallback_persist,
        circuit_fallback_sqm: sg_config.circuit_fallback_sqm.trim().to_ascii_lowercase(),
        delay_threshold_ms: sg_config.delay_threshold_ms,
        delay_threshold_ratio: sg_config.delay_threshold_ratio,
        baseline_alpha_up: sg_config.baseline_alpha_up,
        baseline_alpha_down: sg_config.baseline_alpha_down,
        probe_interval_seconds: sg_config.probe_interval_seconds,
        min_throughput_mbps_for_rtt: sg_config.min_throughput_mbps_for_rtt,
        active_ping_target: sg_config.active_ping_target.clone(),
        active_ping_interval_seconds: sg_config.active_ping_interval_seconds,
        active_ping_weight: sg_config.active_ping_weight,
        active_ping_timeout_seconds: sg_config.active_ping_timeout_seconds,
    };

    Ok(result)
}

fn load_stormguard_site_overrides() -> HashMap<String, (Option<u32>, Option<u32>)> {
    let Ok(overrides) = OverrideStore::load_layer(OverrideLayer::Stormguard) else {
        warn!("Unable to load StormGuard override layer; starting from planned rates.");
        return HashMap::new();
    };

    overrides
        .network_adjustments()
        .iter()
        .filter_map(|adj| match adj {
            NetworkAdjustment::AdjustSiteSpeed {
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
            } => Some((
                site_name.clone(),
                (*download_bandwidth_mbps, *upload_bandwidth_mbps),
            )),
            _ => None,
        })
        .collect()
}

fn get_sites_from_queueing_structure(
    sg_config: &lqos_config::StormguardConfig,
    persisted_site_overrides: &HashMap<String, (Option<u32>, Option<u32>)>,
) -> HashMap<String, WatchingSite> {
    let mut selected: Vec<String> = if sg_config.all_sites {
        all_candidate_site_names()
    } else {
        sg_config.targets.clone()
    };

    let excluded: HashSet<&str> = sg_config
        .exclude_sites
        .iter()
        .map(|site| site.as_str())
        .collect();

    selected.retain(|site| !excluded.contains(site.as_str()));
    selected.sort();
    selected.dedup();

    let mut sites = HashMap::new();
    {
        let mut lock = STORMGUARD_STATS.lock();
        lock.clear();
    }

    for target in selected {
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
        let persisted = persisted_site_overrides
            .get(&target)
            .copied()
            .unwrap_or((None, None));
        let current_download_mbps = persisted
            .0
            .map(u64::from)
            .unwrap_or(max_down)
            .clamp(min_down, max_down);
        let current_upload_mbps = persisted
            .1
            .map(u64::from)
            .unwrap_or(max_up)
            .clamp(min_up, max_up);

        let site = WatchingSite {
            name: target.to_owned(),
            max_download_mbps: max_down,
            max_upload_mbps: max_up,
            min_download_mbps: min_down,
            min_upload_mbps: min_up,
            dependent_nodes: dependencies,
            current_download_mbps,
            current_upload_mbps,
        };
        sites.insert(target.to_owned(), site);
        {
            let mut lock = STORMGUARD_STATS.lock();
            lock.push((
                target.to_owned(),
                current_download_mbps,
                current_upload_mbps,
            ));
        }
    }
    sites
}
