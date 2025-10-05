use crate::uisp_types::DetectedAccessPoint;
use crate::uisp_types::uisp_site_type::UispSiteType;
use lqos_config::Config;
use std::collections::HashSet;
use tracing::warn;
use uisp::{DataLink, Device, Site};

/// Shortened/flattened version of the UISP Site type.
#[derive(Debug)]
pub struct UispSite {
    pub id: String,
    pub name: String,
    pub site_type: UispSiteType,
    pub uisp_parent_id: Option<String>,
    pub parent_indices: HashSet<usize>,
    pub max_down_mbps: u64,
    pub max_up_mbps: u64,
    // Subscriber QoS (from UISP qos), Mbps
    pub base_down_mbps: f32,
    pub base_up_mbps: f32,
    pub burst_down_mbps: f32,
    pub burst_up_mbps: f32,
    pub suspended: bool,
    pub device_indices: Vec<usize>,
    pub route_weights: Vec<(usize, u32)>,
    pub selected_parent: Option<usize>,
}

impl Default for UispSite {
    fn default() -> Self {
        Self {
            id: "".to_string(),
            name: "".to_string(),
            site_type: UispSiteType::Site,
            uisp_parent_id: None,
            parent_indices: Default::default(),
            max_down_mbps: 0,
            max_up_mbps: 0,
            base_down_mbps: 0.0,
            base_up_mbps: 0.0,
            burst_down_mbps: 0.0,
            burst_up_mbps: 0.0,
            suspended: false,
            device_indices: Vec::new(),
            route_weights: Vec::new(),
            selected_parent: None,
        }
    }
}

impl UispSite {
    /// Converts a UISP Site into a UispSite.
    pub fn from_uisp(value: &Site, config: &Config) -> Self {
        let mut uisp_parent_id = None;

        if let Some(id) = &value.identification {
            if let Some(parent) = &id.parent {
                if let Some(pid) = &parent.id {
                    uisp_parent_id = Some(pid.clone());
                }
            }
            if let Some(status) = &id.status {
                if status == "disconnected" {
                    warn!("Site {:?} is disconnected.", id.name);
                }
            }
        }

        let (mut max_down_mbps, mut max_up_mbps) = value.qos(
            config.queues.generated_pn_download_mbps,
            config.queues.generated_pn_upload_mbps,
        );
        // Extract UISP QoS base speeds (bps -> Mbps) and burst sizes (kB/s -> Mbps)
        let mut base_down_mbps: f32 = 0.0;
        let mut base_up_mbps: f32 = 0.0;
        let mut burst_down_mbps: f32 = 0.0;
        let mut burst_up_mbps: f32 = 0.0;
        if let Some(qos) = &value.qos {
            if let Some(d) = qos.downloadSpeed {
                base_down_mbps = (d as f32) / 1_000_000.0;
            }
            if let Some(u) = qos.uploadSpeed {
                base_up_mbps = (u as f32) / 1_000_000.0;
            }
            if let Some(db) = qos.downloadBurstSize {
                burst_down_mbps = (db as f32) * 8.0 / 1000.0 / 1024.0;
            }
            if let Some(ub) = qos.uploadBurstSize {
                burst_up_mbps = (ub as f32) * 8.0 / 1000.0 / 1024.0;
            }
        }
        let suspended = value.is_suspended();

        if suspended {
            match config.uisp_integration.suspended_strategy.as_str() {
                "slow" => {
                    warn!(
                        "{} is suspended. Using slow strategy.",
                        value.name_or_blank()
                    );
                    // Keep capacity minimal for infra calculations
                    max_down_mbps = 1;
                    max_up_mbps = 1;
                    // Base and burst will be ignored by writers (set to 0, so fallback path uses suspended override)
                    base_down_mbps = 0.0;
                    base_up_mbps = 0.0;
                    burst_down_mbps = 0.0;
                    burst_up_mbps = 0.0;
                }
                _ => warn!(
                    "{} is suspended. No strategy is set, leaving at full speed.",
                    value.name_or_blank()
                ),
            }
        }

        Self {
            id: value.id.clone(),
            name: value.name_or_blank(),
            site_type: UispSiteType::from_uisp_record(value).unwrap(),
            parent_indices: HashSet::new(),
            uisp_parent_id,
            max_down_mbps,
            max_up_mbps,
            base_down_mbps,
            base_up_mbps,
            burst_down_mbps,
            burst_up_mbps,
            suspended,
            ..Default::default()
        }
    }

    /// Compute burst-aware min/max in Mbps using UISP qos + config multipliers.
    /// Returns None if no qos base rates are present.
    pub fn burst_rates(&self, config: &Config) -> Option<(f32, f32, f32, f32)> {
        // Suspended slow: override to 0.1/0.1 irrespective of multipliers/floors
        if self.suspended && config.uisp_integration.suspended_strategy == "slow" {
            return Some((0.1, 0.1, 0.1, 0.1));
        }
        if self.base_down_mbps <= 0.0 && self.base_up_mbps <= 0.0 {
            return None;
        }
        let dl_min = self.base_down_mbps * config.uisp_integration.commit_bandwidth_multiplier;
        let ul_min = self.base_up_mbps * config.uisp_integration.commit_bandwidth_multiplier;
        let dl_max = (self.base_down_mbps + self.burst_down_mbps)
            * config.uisp_integration.bandwidth_overhead_factor;
        let ul_max = (self.base_up_mbps + self.burst_up_mbps)
            * config.uisp_integration.bandwidth_overhead_factor;
        Some((dl_min, dl_max, ul_min, ul_max))
    }

    pub fn find_aps(
        &self,
        devices: &[Device],
        data_links: &[DataLink],
        sites: &[Site],
    ) -> Vec<DetectedAccessPoint> {
        let mut links = Vec::new();

        for device in devices.iter() {
            if let Some(device_site) = device.get_site_id() {
                if device_site == self.id {
                    // We're in the correct site, now look for anything that
                    // links to/from this device
                    let potential_ap_id = device.get_id();
                    let mut potential_ap = DetectedAccessPoint {
                        site_id: self.id.clone(),
                        device_id: potential_ap_id.clone(),
                        device_name: device.get_name().unwrap_or(String::new()),
                        child_sites: vec![],
                    };

                    for dl in data_links.iter() {
                        // The "I'm the FROM device case"
                        if let Some(from_device) = &dl.from.device {
                            if from_device.identification.id == potential_ap_id {
                                if let Some(to_site) = &dl.to.site {
                                    if to_site.identification.id != self.id {
                                        // We have a data link from this device that goes to
                                        // another site.
                                        if let Some(remote_site) =
                                            sites.iter().find(|s| s.id == to_site.identification.id)
                                        {
                                            potential_ap.child_sites.push(remote_site.id.clone());
                                        }
                                    }
                                }
                            }
                        }

                        // The "I'm the TO the device case"
                        if let Some(to_device) = &dl.to.device {
                            if to_device.identification.id == potential_ap_id {
                                if let Some(from_site) = &dl.from.site {
                                    if from_site.identification.id != self.id {
                                        // We have a data link from this device that goes to
                                        // another site.
                                        if let Some(remote_site) = sites
                                            .iter()
                                            .find(|s| s.id == from_site.identification.id)
                                        {
                                            potential_ap.child_sites.push(remote_site.id.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !potential_ap.child_sites.is_empty() {
                        links.push(potential_ap);
                    }
                }
            }
        }
        links
    }

    pub fn print_tree_summary(&self) {
        print!(
            "{} ({}) {}/{} Mbps",
            self.name, self.site_type, self.max_down_mbps, self.max_up_mbps
        );
        if self.suspended {
            print!(" (SUSPENDED)");
        }
        if !self.device_indices.is_empty() {
            print!(" [{} devices]", self.device_indices.len());
        }
    }
}
