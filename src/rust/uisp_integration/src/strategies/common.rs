use crate::blackboard_blob;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::full::parse::parse_uisp_datasets;
use crate::strategies::full::uisp_fetch::load_uisp_data;
use crate::uisp_types::{UispDevice, UispSite, UispSiteType};
use lqos_config::Config;
use std::collections::HashMap;
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

impl UispData {
    pub(crate) async fn fetch_uisp_data(
        config: Arc<Config>,
        ip_ranges: IpRanges,
    ) -> std::result::Result<Self, UispIntegrationError> {
        // Obtain the UISP data and transform it into easier to work with types
        let (sites_raw, devices_raw, data_links_raw) = load_uisp_data(config.clone()).await?;

        if let Err(e) = blackboard_blob("uisp_sites", &sites_raw).await {
            warn!("Unable to write sites to blackboard: {e:?}");
        }
        if let Err(e) = blackboard_blob("uisp_devices", &devices_raw).await {
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
        let mut mappings = HashMap::new();

        let mut devices_by_site: HashMap<String, Vec<&Device>> = HashMap::new();
        let mut device_by_id: HashMap<String, &Device> = HashMap::new();
        for device in self.devices_raw.iter() {
            if let Some(site_id) = device.get_site_id() {
                devices_by_site.entry(site_id).or_default().push(device);
            }
            device_by_id.insert(device.identification.id.clone(), device);
        }

        let mut links_from_device: HashMap<String, Vec<&DataLink>> = HashMap::new();
        let mut links_to_device: HashMap<String, Vec<&DataLink>> = HashMap::new();
        let mut links_from_site: HashMap<String, Vec<&DataLink>> = HashMap::new();
        let mut links_to_site: HashMap<String, Vec<&DataLink>> = HashMap::new();
        for link in self.data_links_raw.iter() {
            if let Some(device) = &link.from.device {
                links_from_device
                    .entry(device.identification.id.clone())
                    .or_default()
                    .push(link);
            }
            if let Some(device) = &link.to.device {
                links_to_device
                    .entry(device.identification.id.clone())
                    .or_default()
                    .push(link);
            }
            if let Some(site) = &link.from.site {
                links_from_site
                    .entry(site.identification.id.clone())
                    .or_default()
                    .push(link);
            }
            if let Some(site) = &link.to.site {
                links_to_site
                    .entry(site.identification.id.clone())
                    .or_default()
                    .push(link);
            }
        }

        for client in self.find_client_sites() {
            let mut found = false;
            let mut parent: Option<String> = None;
            if let Some(devices) = devices_by_site.get(&client.id) {
                'device_search: for device in devices.iter() {
                    if let Some(attr) = &device.attributes {
                        if let Some(ap) = &attr.apDevice {
                            if let Some(ap_id) = &ap.id {
                                if let Some(apdev) = device_by_id.get(ap_id) {
                                    if apdev.get_site_id().unwrap_or_default() != client.id {
                                        parent = Some(apdev.identification.id.clone());
                                        found = true;
                                        break 'device_search;
                                    }
                                }
                            }
                        }
                    }

                    if let Some(links) = links_from_device.get(&device.identification.id) {
                        for link in links {
                            if let Some(to_device) = &link.to.device {
                                if let Some(apdev) = device_by_id.get(&to_device.identification.id)
                                {
                                    if apdev.get_site_id().unwrap_or_default() != client.id {
                                        parent = Some(apdev.identification.id.clone());
                                        found = true;
                                        break 'device_search;
                                    }
                                }
                            }
                        }
                    }
                    if let Some(links) = links_to_device.get(&device.identification.id) {
                        for link in links {
                            if let Some(from_device) = &link.from.device {
                                if let Some(apdev) =
                                    device_by_id.get(&from_device.identification.id)
                                {
                                    if apdev.get_site_id().unwrap_or_default() != client.id {
                                        parent = Some(apdev.identification.id.clone());
                                        found = true;
                                        break 'device_search;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !found {
                if let Some(links) = links_from_site.get(&client.id) {
                    for link in links {
                        if let Some(to_device) = &link.to.device {
                            if let Some(apdev) = device_by_id.get(&to_device.identification.id) {
                                if apdev.get_site_id().unwrap_or_default() != client.id {
                                    parent = Some(apdev.identification.id.clone());
                                    found = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            if !found {
                if let Some(links) = links_to_site.get(&client.id) {
                    for link in links {
                        if let Some(from_device) = &link.from.device {
                            if let Some(apdev) = device_by_id.get(&from_device.identification.id) {
                                if apdev.get_site_id().unwrap_or_default() != client.id {
                                    parent = Some(apdev.identification.id.clone());
                                    found = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            if !found {
                mappings
                    .entry("Orphans".to_string())
                    .or_insert_with(Vec::new)
                    .push(client.id.clone());
            } else if let Some(parent_id) = parent {
                mappings
                    .entry(parent_id)
                    .or_insert_with(Vec::new)
                    .push(client.id.clone());
            }
        }
        mappings
    }

    pub fn map_clients_to_aps_by_name(&self) -> HashMap<String, Vec<String>> {
        let mut name_map = HashMap::new();
        for (ap_id, client_ids) in self.map_clients_to_aps() {
            if ap_id == "Orphans" {
                name_map.insert(ap_id, client_ids);
                continue;
            }

            if let Some(device) = self.devices.iter().find(|d| d.id == ap_id) {
                name_map.insert(device.name.clone(), client_ids);
            } else if let Some(device) = self
                .devices_raw
                .iter()
                .find(|d| d.identification.id == ap_id)
            {
                let display = device.get_name().unwrap_or(ap_id.clone());
                name_map.insert(display, client_ids);
            } else {
                name_map.insert(ap_id, client_ids);
            }
        }
        name_map
    }
}
