use crate::uisp_types::uisp_site_type::UispSiteType;
use crate::uisp_types::DetectedAccessPoint;
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
    pub max_down_mbps: u32,
    pub max_up_mbps: u32,
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
        let suspended = value.is_suspended();

        if suspended {
            match config.uisp_integration.suspended_strategy.as_str() {
                "slow" => {
                    warn!(
                        "{} is suspended. Setting a slow speed.",
                        value.name_or_blank()
                    );
                    max_down_mbps = 1;
                    max_up_mbps = 1;
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
            suspended,
            ..Default::default()
        }
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
