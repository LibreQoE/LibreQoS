use crate::blackboard_blob;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::full::parse::parse_uisp_datasets;
use crate::strategies::full::uisp_fetch::load_uisp_data;
use crate::uisp_types::{UispDevice, UispSite, UispSiteType};
use lqos_config::Config;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::warn;
use uisp::{DataLink, Device, Site};

pub(crate) struct UispData {
    pub sites_raw: Vec<Site>,
    pub devices_raw: Vec<Device>,
    pub data_links_raw: Vec<DataLink>,
    pub sites: Vec<UispSite>,
    pub devices: Vec<UispDevice>,
    //pub data_links: Vec<UispDataLink>,
}

/// Ensure site names are unique by appending the site ID to duplicates.
pub(crate) fn dedup_site_names(sites: &mut Vec<Site>) {
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for site in sites.iter() {
        let base_name = site.name_or_blank();
        if base_name.is_empty() {
            continue;
        }
        *name_counts.entry(base_name).or_insert(0) += 1;
    }

    for site in sites.iter_mut() {
        let base_name = site.name_or_blank();
        if base_name.is_empty() {
            continue;
        }

        if name_counts.get(&base_name).copied().unwrap_or(0) > 1 {
            let candidate = format!("{base_name} {}", site.id);
            if let Some(ident) = site.identification.as_mut() {
                ident.name = Some(candidate.clone());
            }
            warn!(
                site_id = %site.id,
                original = %base_name,
                renamed = %candidate,
                "Duplicate UISP site name detected; renaming with site ID for LibreQoS uniqueness"
            );
        }
    }
}

impl UispData {
    pub(crate) async fn fetch_uisp_data(
        config: Arc<Config>,
        ip_ranges: IpRanges,
    ) -> std::result::Result<Self, UispIntegrationError> {
        // Obtain the UISP data and transform it into easier to work with types
        let (mut sites_raw, devices_raw, data_links_raw, devices_as_json) =
            load_uisp_data(config.clone()).await?;

        // Deduplicate site names so downstream graph building has unique keys
        dedup_site_names(&mut sites_raw);

        if let Err(e) = blackboard_blob("uisp_sites", &sites_raw).await {
            warn!("Unable to write sites to blackboard: {e:?}");
        }
        if let Err(e) = blackboard_blob("uisp_devices", &devices_as_json).await {
            warn!("Unable to write devices to blackboard: {e:?}");
        }
        if let Err(e) = blackboard_blob("uisp_data_links", &data_links_raw).await {
            warn!("Unable to write data links to blackboard: {e:?}");
        }

        // If Mikrotik is enabled, we need to fetch the Mikrotik data
        let ipv4_to_v6 = crate::strategies::full::mikrotik::mikrotik_data(&config)
            .await
            .unwrap_or_else(|_| Vec::new());

        // Parse the UISP data into a more usable format
        let (sites, _data_links, devices) = parse_uisp_datasets(
            &sites_raw,
            &data_links_raw,
            &devices_raw,
            &config,
            &ip_ranges,
            ipv4_to_v6,
        );

        Ok(UispData {
            sites_raw,
            devices_raw,
            data_links_raw,
            sites,
            devices,
            //data_links: _data_links,
        })
    }

    pub fn find_client_sites(&self) -> Vec<&UispSite> {
        self.sites
            .iter()
            .filter(|s| s.site_type == UispSiteType::Client)
            .collect()
    }

    pub fn find_devices_in_site(&self, site_id: &str) -> Vec<&Device> {
        self.devices_raw
            .iter()
            .filter(|d| d.get_site_id().unwrap_or_default() == site_id)
            .collect()
    }

    pub fn find_device_by_id(&self, device_id: &str) -> Option<&Device> {
        self.devices_raw
            .iter()
            .find(|d| d.identification.id == device_id)
    }

    pub fn find_device_by_name(&self, device_name: &str) -> Option<&Device> {
        self.devices_raw
            .iter()
            .find(|d| d.get_name().unwrap_or_default() == device_name)
    }

    pub fn map_clients_to_aps(&self) -> HashMap<String, Vec<String>> {
        let mut mappings: HashMap<String, HashSet<String>> = HashMap::new();
        for client in self.find_client_sites() {
            let mut found = false;
            let mut parent = None;
            for device in self.find_devices_in_site(&client.id) {
                //println!("Client {} has a device {:?}", client.name, device.get_name());
                // Look for Parent AP attributes
                if let Some(attr) = &device.attributes {
                    if let Some(ap) = &attr.apDevice {
                        if let Some(ap_id) = &ap.id {
                            //println!("AP ID: {}", ap_id);
                            if let Some(apdev) = self.find_device_by_id(ap_id) {
                                //println!("AP Device: {:?}", apdev.get_name());
                                if apdev.get_site_id().unwrap_or_default() != client.id {
                                    parent = Some(("AP", apdev.identification.id.clone()));
                                    found = true;
                                }
                            }
                        }
                    }
                }

                // Look in Site-DeviceSite
                // NOTE: This block was removed because device_site.id is a site ID, not a device ID
                // and cannot be used with find_device_by_id()

                // Look for data links with this device
                if !found {
                    for link in self.data_links_raw.iter() {
                        // Check the FROM side
                        if let Some(from_device) = &link.from.device {
                            if from_device.identification.id == device.identification.id {
                                if let Some(to_device) = &link.to.device {
                                    if let Some(apdev) =
                                        self.find_device_by_id(&to_device.identification.id)
                                    {
                                        if apdev.get_site_id().unwrap_or_default() != client.id {
                                            parent = Some(("AP", apdev.identification.id.clone()));
                                            found = true;
                                        }
                                    }
                                }
                            }
                        }
                        // Check the TO side
                        if let Some(to_device) = &link.to.device {
                            if to_device.identification.id == device.identification.id {
                                if let Some(from_device) = &link.from.device {
                                    if let Some(apdev) =
                                        self.find_device_by_id(&from_device.identification.id)
                                    {
                                        if apdev.get_site_id().unwrap_or_default() != client.id {
                                            parent = Some(("AP", apdev.identification.id.clone()));
                                            found = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // If we still haven't found anything, let's try data links to the client site as a whole
            if !found {
                for link in self.data_links_raw.iter() {
                    if let Some(from_site) = &link.from.site {
                        if from_site.identification.id == client.id {
                            if let Some(to_device) = &link.to.device {
                                if let Some(apdev) =
                                    self.find_device_by_id(&to_device.identification.id)
                                {
                                    if apdev.get_site_id().unwrap_or_default() != client.id {
                                        parent = Some(("AP", apdev.identification.id.clone()));
                                        found = true;
                                    }
                                }
                            }
                        }
                    }
                    if let Some(to_site) = &link.to.site {
                        if to_site.identification.id == client.id {
                            if let Some(from_device) = &link.from.device {
                                if let Some(apdev) =
                                    self.find_device_by_id(&from_device.identification.id)
                                {
                                    if apdev.get_site_id().unwrap_or_default() != client.id {
                                        parent = Some(("AP", apdev.identification.id.clone()));
                                        found = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !found {
                //println!("Client {} has no obvious parent AP", client.name);
                let entry = mappings
                    .entry("Orphans".to_string())
                    .or_insert_with(HashSet::new);
                entry.insert(client.id.clone());
            } else {
                //info!("Client {} is connected to {:?}", client.name, parent);
                if let Some((_, parent)) = &parent {
                    let entry = mappings
                        .entry(parent.to_string())
                        .or_insert_with(HashSet::new);
                    entry.insert(client.id.clone());
                }
            }
        }
        mappings
            .into_iter()
            .map(|(ap, sites)| {
                let mut sites_vec: Vec<_> = sites.into_iter().collect();
                sites_vec.sort();
                (ap, sites_vec)
            })
            .collect()
    }
}
