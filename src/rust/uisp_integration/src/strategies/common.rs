use crate::blackboard_blob;
use crate::errors::UispIntegrationError;
use crate::ip_ranges::IpRanges;
use crate::strategies::full::parse::parse_uisp_datasets;
use crate::strategies::full::uisp_fetch::load_uisp_data;
use crate::uisp_types::{UispDevice, UispSite, UispSiteType};
use lqos_config::Config;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};
use uisp::{DataLink, Device, Site};

pub(crate) struct UispData {
    pub sites_raw: Vec<Site>,
    pub devices_raw: Vec<Device>,
    pub data_links_raw: Vec<DataLink>,
    pub sites: Vec<UispSite>,
    pub devices: Vec<UispDevice>,
    raw_device_index_by_id: HashMap<String, usize>,
    raw_device_indices_by_site_id: HashMap<String, Vec<usize>>,
    parsed_device_index_by_id: HashMap<String, usize>,
    parsed_device_indices_by_site_id: HashMap<String, Vec<usize>>,
    site_index_by_id: HashMap<String, usize>,
    data_link_indices_by_device_id: HashMap<String, Vec<usize>>,
    data_link_indices_by_site_id: HashMap<String, Vec<usize>>,
}

fn short_address_segment(site: &Site) -> Option<String> {
    let address = site.address()?;
    let first_segment = address.split(',').next()?.trim();
    if first_segment.is_empty() {
        None
    } else {
        Some(first_segment.to_string())
    }
}

fn service_name(site: &Site) -> Option<String> {
    let name = site.ucrm.as_ref()?.service.as_ref()?.name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn short_site_id(site: &Site) -> String {
    site.id.chars().take(8).collect()
}

fn looks_like_business_name_part(part: &str) -> bool {
    let normalized = part.trim().trim_end_matches('.').to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "llc"
            | "inc"
            | "corp"
            | "corporation"
            | "co"
            | "company"
            | "ltd"
            | "pllc"
            | "llp"
            | "lp"
            | "pc"
            | "plc"
    )
}

fn normalized_client_personal_name(name: &str) -> Option<String> {
    let mut parts = name.split(',');
    let last_name = parts.next()?.trim();
    let first_name = parts.next()?.trim();
    if parts.next().is_some() || last_name.is_empty() || first_name.is_empty() {
        return None;
    }

    if last_name.chars().any(|c| c.is_ascii_digit())
        || first_name.chars().any(|c| c.is_ascii_digit())
    {
        return None;
    }

    if looks_like_business_name_part(last_name) || looks_like_business_name_part(first_name) {
        return None;
    }

    Some(format!("{first_name} {last_name}"))
}

fn normalize_client_site_names(sites: &mut [Site]) {
    let mut normalized_count = 0usize;
    let mut sample_names = Vec::new();

    for site in sites.iter_mut().filter(|site| site.is_client_site()) {
        let original_name = site.name_or_blank();
        let Some(normalized_name) = normalized_client_personal_name(&original_name) else {
            continue;
        };

        if normalized_name == original_name {
            continue;
        }

        if let Some(ident) = site.identification.as_mut() {
            ident.name = Some(normalized_name.clone());
        }
        normalized_count += 1;
        if sample_names.len() < 3 {
            sample_names.push(format!("{original_name} -> {normalized_name}"));
        }
    }

    if normalized_count > 0 {
        info!(
            normalized_sites = normalized_count,
            sample = ?sample_names,
            "Normalized UISP client site names from last-name-first format"
        );
    }
}

fn disambiguation_candidates(site: &Site, base_name: &str) -> Vec<String> {
    let mut candidates = Vec::new();

    if let Some(address_segment) = short_address_segment(site)
        && address_segment != base_name
    {
        candidates.push(format!("{base_name} ({address_segment})"));
    }

    if let Some(service_name) = service_name(site)
        && service_name != base_name
    {
        candidates.push(format!("{base_name} ({service_name})"));
    }

    let short_id = short_site_id(site);
    if !short_id.is_empty() {
        candidates.push(format!("{base_name} ({short_id})"));
    }
    candidates.push(format!("{base_name} ({})", site.id));

    candidates
}

/// Ensure site names are unique by appending a human-friendly disambiguator to duplicates.
pub(crate) fn dedup_site_names(sites: &mut [Site]) {
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for site in sites.iter() {
        let base_name = site.name_or_blank();
        if base_name.is_empty() {
            continue;
        }
        *name_counts.entry(base_name).or_insert(0) += 1;
    }

    let mut used_names: HashSet<String> = sites
        .iter()
        .filter_map(|site| {
            let base_name = site.name_or_blank();
            if base_name.is_empty() || name_counts.get(&base_name).copied().unwrap_or(0) > 1 {
                None
            } else {
                Some(base_name)
            }
        })
        .collect();

    for site in sites.iter_mut() {
        let base_name = site.name_or_blank();
        if base_name.is_empty() {
            continue;
        }

        if name_counts.get(&base_name).copied().unwrap_or(0) > 1 {
            let candidate = disambiguation_candidates(site, &base_name)
                .into_iter()
                .find(|candidate| !used_names.contains(candidate))
                .unwrap_or_else(|| format!("{base_name} ({})", site.id));
            used_names.insert(candidate.clone());
            if let Some(ident) = site.identification.as_mut() {
                ident.name = Some(candidate.clone());
            }
            warn!(
                site_id = %site.id,
                original = %base_name,
                renamed = %candidate,
                "Duplicate UISP site name detected; renaming with human-friendly disambiguator for LibreQoS uniqueness"
            );
        }
    }
}

fn dedup_raw_devices_by_id(
    devices_raw: Vec<Device>,
    devices_as_json: Vec<Value>,
) -> (Vec<Device>, Vec<Value>) {
    let original_raw_len = devices_raw.len();
    let original_json_len = devices_as_json.len();
    if original_raw_len != original_json_len {
        warn!(
            raw_devices = original_raw_len,
            raw_device_json = original_json_len,
            "UISP device rows and raw device JSON rows differ in length before dedupe"
        );
    }

    let mut seen_ids = HashSet::<String>::new();
    let mut duplicate_counts = HashMap::<String, usize>::new();
    let mut deduped_devices = Vec::with_capacity(original_raw_len);
    let mut deduped_json = Vec::with_capacity(original_json_len);

    for (device, raw_json) in devices_raw.into_iter().zip(devices_as_json.into_iter()) {
        let device_id = device.identification.id.clone();
        if !seen_ids.insert(device_id.clone()) {
            *duplicate_counts.entry(device_id).or_insert(0) += 1;
            continue;
        }
        deduped_devices.push(device);
        deduped_json.push(raw_json);
    }

    if !duplicate_counts.is_empty() {
        let duplicate_rows = duplicate_counts.values().sum::<usize>();
        let mut duplicate_ids = duplicate_counts.into_iter().collect::<Vec<_>>();
        duplicate_ids.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        let sample_ids = duplicate_ids
            .iter()
            .take(5)
            .map(|(device_id, count)| format!("{device_id} (+{count})"))
            .collect::<Vec<_>>();
        warn!(
            duplicate_device_ids = duplicate_ids.len(),
            duplicate_device_rows = duplicate_rows,
            sample_ids = ?sample_ids,
            "UISP returned duplicate device IDs; deduping by device ID before topology processing"
        );
    }

    (deduped_devices, deduped_json)
}

impl UispData {
    pub(crate) fn from_parts(
        sites_raw: Vec<Site>,
        devices_raw: Vec<Device>,
        data_links_raw: Vec<DataLink>,
        sites: Vec<UispSite>,
        devices: Vec<UispDevice>,
    ) -> Self {
        let mut raw_device_index_by_id = HashMap::new();
        let mut raw_device_indices_by_site_id = HashMap::<String, Vec<usize>>::new();
        for (index, device) in devices_raw.iter().enumerate() {
            raw_device_index_by_id.insert(device.identification.id.clone(), index);
            if let Some(site_id) = device.get_site_id() {
                raw_device_indices_by_site_id
                    .entry(site_id.to_string())
                    .or_default()
                    .push(index);
            }
        }

        let mut parsed_device_index_by_id = HashMap::new();
        let mut parsed_device_indices_by_site_id = HashMap::<String, Vec<usize>>::new();
        for (index, device) in devices.iter().enumerate() {
            parsed_device_index_by_id.insert(device.id.clone(), index);
            parsed_device_indices_by_site_id
                .entry(device.site_id.clone())
                .or_default()
                .push(index);
        }

        let site_index_by_id = sites
            .iter()
            .enumerate()
            .map(|(index, site)| (site.id.clone(), index))
            .collect::<HashMap<_, _>>();

        let mut data_link_indices_by_device_id = HashMap::<String, Vec<usize>>::new();
        let mut data_link_indices_by_site_id = HashMap::<String, Vec<usize>>::new();
        for (index, link) in data_links_raw.iter().enumerate() {
            if let Some(device) = link.from.device.as_ref() {
                data_link_indices_by_device_id
                    .entry(device.identification.id.clone())
                    .or_default()
                    .push(index);
            }
            if let Some(device) = link.to.device.as_ref() {
                data_link_indices_by_device_id
                    .entry(device.identification.id.clone())
                    .or_default()
                    .push(index);
            }
            if let Some(site) = link.from.site.as_ref() {
                data_link_indices_by_site_id
                    .entry(site.identification.id.clone())
                    .or_default()
                    .push(index);
            }
            if let Some(site) = link.to.site.as_ref() {
                data_link_indices_by_site_id
                    .entry(site.identification.id.clone())
                    .or_default()
                    .push(index);
            }
        }

        Self {
            sites_raw,
            devices_raw,
            data_links_raw,
            sites,
            devices,
            raw_device_index_by_id,
            raw_device_indices_by_site_id,
            parsed_device_index_by_id,
            parsed_device_indices_by_site_id,
            site_index_by_id,
            data_link_indices_by_device_id,
            data_link_indices_by_site_id,
        }
    }

    pub(crate) async fn fetch_uisp_data(
        config: Arc<Config>,
        ip_ranges: IpRanges,
    ) -> std::result::Result<Self, UispIntegrationError> {
        let fetch_started = Instant::now();
        // Obtain the UISP data and transform it into easier to work with types
        let (mut sites_raw, devices_raw, data_links_raw, devices_as_json) =
            load_uisp_data(config.clone()).await?;
        let (devices_raw, devices_as_json) = dedup_raw_devices_by_id(devices_raw, devices_as_json);

        // Normalize endpoint/customer names before deduplication and downstream parsing.
        normalize_client_site_names(&mut sites_raw);

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

        info!(
            sites = sites.len(),
            devices = devices.len(),
            raw_devices = devices_raw.len(),
            data_links = data_links_raw.len(),
            elapsed_ms = fetch_started.elapsed().as_millis(),
            "Fetched and indexed UISP datasets"
        );

        Ok(Self::from_parts(
            sites_raw,
            devices_raw,
            data_links_raw,
            sites,
            devices,
        ))
    }

    pub fn find_client_sites(&self) -> Vec<&UispSite> {
        self.sites
            .iter()
            .filter(|s| s.site_type == UispSiteType::Client)
            .collect()
    }

    pub fn find_devices_in_site(&self, site_id: &str) -> Vec<&Device> {
        self.raw_device_indices_by_site_id
            .get(site_id)
            .into_iter()
            .flat_map(|indices| indices.iter())
            .filter_map(|index| self.devices_raw.get(*index))
            .collect()
    }

    pub fn find_device_by_id(&self, device_id: &str) -> Option<&Device> {
        self.raw_device_index_by_id
            .get(device_id)
            .and_then(|index| self.devices_raw.get(*index))
    }

    pub fn find_parsed_device_by_id(&self, device_id: &str) -> Option<&UispDevice> {
        self.parsed_device_index_by_id
            .get(device_id)
            .and_then(|index| self.devices.get(*index))
    }

    pub fn find_site_by_id(&self, site_id: &str) -> Option<&UispSite> {
        self.site_index_by_id
            .get(site_id)
            .and_then(|index| self.sites.get(*index))
    }

    pub fn find_parsed_devices_in_site(&self, site_id: &str) -> Vec<&UispDevice> {
        self.parsed_device_indices_by_site_id
            .get(site_id)
            .into_iter()
            .flat_map(|indices| indices.iter())
            .filter_map(|index| self.devices.get(*index))
            .collect()
    }

    pub fn find_uisp_device_by_id(&self, device_id: &str) -> Option<&UispDevice> {
        self.find_parsed_device_by_id(device_id)
    }

    pub fn device_display_name(&self, device_id: &str) -> String {
        self.find_uisp_device_by_id(device_id)
            .map(|device| {
                if device.name.trim().is_empty() {
                    device.id.clone()
                } else {
                    device.name.clone()
                }
            })
            .unwrap_or_else(|| device_id.to_string())
    }

    pub fn map_clients_to_aps(&self) -> HashMap<String, Vec<String>> {
        let started = Instant::now();
        let client_site_count = self.find_client_sites().len();
        let mut mappings: HashMap<String, HashSet<String>> = HashMap::new();
        for client in self.find_client_sites() {
            let mut found = false;
            let mut parent = None;
            for device in self.find_devices_in_site(&client.id) {
                //println!("Client {} has a device {:?}", client.name, device.get_name());
                // Look for Parent AP attributes
                if let Some(attr) = &device.attributes
                    && let Some(ap) = &attr.apDevice
                    && let Some(ap_id) = &ap.id
                {
                    //println!("AP ID: {}", ap_id);
                    if let Some(apdev) = self.find_device_by_id(ap_id) {
                        //println!("AP Device: {:?}", apdev.get_name());
                        if apdev.get_site_id().unwrap_or_default() != client.id {
                            parent = Some(("AP", apdev.identification.id.clone()));
                            found = true;
                        }
                    }
                }

                // Look in Site-DeviceSite
                // NOTE: This block was removed because device_site.id is a site ID, not a device ID
                // and cannot be used with find_device_by_id()

                // Look for data links with this device
                if !found {
                    for link in self
                        .data_link_indices_by_device_id
                        .get(&device.identification.id)
                        .into_iter()
                        .flat_map(|indices| indices.iter())
                        .filter_map(|index| self.data_links_raw.get(*index))
                    {
                        // Check the FROM side
                        if let Some(from_device) = &link.from.device
                            && from_device.identification.id == device.identification.id
                            && let Some(to_device) = &link.to.device
                            && let Some(apdev) =
                                self.find_device_by_id(&to_device.identification.id)
                            && apdev.get_site_id().unwrap_or_default() != client.id
                        {
                            parent = Some(("AP", apdev.identification.id.clone()));
                            found = true;
                        }
                        // Check the TO side
                        if let Some(to_device) = &link.to.device
                            && to_device.identification.id == device.identification.id
                            && let Some(from_device) = &link.from.device
                            && let Some(apdev) =
                                self.find_device_by_id(&from_device.identification.id)
                            && apdev.get_site_id().unwrap_or_default() != client.id
                        {
                            parent = Some(("AP", apdev.identification.id.clone()));
                            found = true;
                        }
                    }
                }
            }

            // If we still haven't found anything, let's try data links to the client site as a whole
            if !found {
                for link in self
                    .data_link_indices_by_site_id
                    .get(&client.id)
                    .into_iter()
                    .flat_map(|indices| indices.iter())
                    .filter_map(|index| self.data_links_raw.get(*index))
                {
                    if let Some(from_site) = &link.from.site
                        && from_site.identification.id == client.id
                        && let Some(to_device) = &link.to.device
                        && let Some(apdev) = self.find_device_by_id(&to_device.identification.id)
                        && apdev.get_site_id().unwrap_or_default() != client.id
                    {
                        parent = Some(("AP", apdev.identification.id.clone()));
                        found = true;
                    }
                    if let Some(to_site) = &link.to.site
                        && to_site.identification.id == client.id
                        && let Some(from_device) = &link.from.device
                        && let Some(apdev) = self.find_device_by_id(&from_device.identification.id)
                        && apdev.get_site_id().unwrap_or_default() != client.id
                    {
                        parent = Some(("AP", apdev.identification.id.clone()));
                        found = true;
                    }
                }
            }

            if !found {
                //println!("Client {} has no obvious parent AP", client.name);
                let entry = mappings.entry("Orphans".to_string()).or_default();
                entry.insert(client.id.clone());
            } else {
                //info!("Client {} is connected to {:?}", client.name, parent);
                if let Some((_, parent)) = &parent {
                    let entry = mappings.entry(parent.to_string()).or_default();
                    entry.insert(client.id.clone());
                }
            }
        }
        let mappings = mappings
            .into_iter()
            .map(|(ap, sites)| {
                let mut sites_vec: Vec<_> = sites.into_iter().collect();
                sites_vec.sort();
                (ap, sites_vec)
            })
            .collect::<HashMap<_, _>>();
        info!(
            client_sites = client_site_count,
            mapped_parents = mappings.len(),
            elapsed_ms = started.elapsed().as_millis(),
            "Mapped UISP client sites to upstream APs"
        );
        mappings
    }
}

#[cfg(test)]
mod tests {
    use super::{dedup_raw_devices_by_id, dedup_site_names, normalize_client_site_names};
    use serde_json::json;
    use uisp::{Device, Site};

    fn mk_site(id: &str, name: &str, address: Option<&str>, service_name: Option<&str>) -> Site {
        let mut value = json!({
            "id": id,
            "identification": {
                "id": id,
                "name": name,
                "type": "endpoint",
                "suspended": false
            }
        });

        if let Some(address) = address {
            value["description"] = json!({ "address": address });
        }
        if let Some(service_name) = service_name {
            value["ucrm"] = json!({
                "service": {
                    "id": format!("svc-{id}"),
                    "name": service_name,
                    "status": 1,
                    "tariffId": "1",
                    "trafficShapingOverrideEnabled": false
                }
            });
        }

        serde_json::from_value(value).expect("site JSON must deserialize")
    }

    fn mk_device(id: &str, name: &str, site_id: &str) -> Device {
        serde_json::from_value(json!({
            "identification": {
                "id": id,
                "hostname": name,
                "role": "ap",
                "site": {
                    "id": site_id,
                    "parent": null
                }
            },
            "overview": {
                "status": "active"
            }
        }))
        .expect("device JSON must deserialize")
    }

    #[test]
    fn duplicate_site_names_prefer_address_segment() {
        let mut sites = vec![
            mk_site(
                "213f4b53-bddf-41dd-af65-2dbaa5bf0927",
                "Pathway Communications",
                Some("7100 Binational Way, Santa Teresa, 88008, New Mexico, United States"),
                None,
            ),
            mk_site(
                "96bb13cd-10cc-43a1-ae56-a389161178d7",
                "Pathway Communications",
                Some("175 Lindburgh Dr, El Paso, 79932, Texas, United States"),
                None,
            ),
        ];

        dedup_site_names(&mut sites);

        assert_eq!(
            sites[0].name_or_blank(),
            "Pathway Communications (7100 Binational Way)"
        );
        assert_eq!(
            sites[1].name_or_blank(),
            "Pathway Communications (175 Lindburgh Dr)"
        );
    }

    #[test]
    fn duplicate_site_names_fall_back_to_service_name() {
        let mut sites = vec![
            mk_site("site-1", "Acme", None, Some("1000/1000 Mbps - Primary")),
            mk_site("site-2", "Acme", None, Some("1000/1000 Mbps - Backup")),
        ];

        dedup_site_names(&mut sites);

        assert_eq!(sites[0].name_or_blank(), "Acme (1000/1000 Mbps - Primary)");
        assert_eq!(sites[1].name_or_blank(), "Acme (1000/1000 Mbps - Backup)");
    }

    #[test]
    fn duplicate_site_names_fall_back_to_short_id() {
        let mut sites = vec![
            mk_site("abcd1234-0000-0000-0000-000000000000", "Acme", None, None),
            mk_site("efgh5678-0000-0000-0000-000000000000", "Acme", None, None),
        ];

        dedup_site_names(&mut sites);

        assert_eq!(sites[0].name_or_blank(), "Acme (abcd1234)");
        assert_eq!(sites[1].name_or_blank(), "Acme (efgh5678)");
    }

    #[test]
    fn client_site_names_reorder_last_first_to_first_last() {
        let mut sites = vec![mk_site("site-1", "Rubio, Jorge", None, None)];

        normalize_client_site_names(&mut sites);

        assert_eq!(sites[0].name_or_blank(), "Jorge Rubio");
    }

    #[test]
    fn client_site_names_leave_business_suffix_names_unchanged() {
        let mut sites = vec![mk_site("site-1", "Acme, LLC", None, None)];

        normalize_client_site_names(&mut sites);

        assert_eq!(sites[0].name_or_blank(), "Acme, LLC");
    }

    #[test]
    fn normalize_client_site_names_skips_non_client_sites() {
        let mut site = mk_site("site-1", "Rubio, Jorge", None, None);
        site.identification
            .as_mut()
            .expect("site must have identification")
            .site_type = Some("site".to_string());
        let mut sites = vec![site];

        normalize_client_site_names(&mut sites);

        assert_eq!(sites[0].name_or_blank(), "Rubio, Jorge");
    }

    #[test]
    fn duplicate_raw_devices_are_deduped_by_id_with_matching_json_rows() {
        let first = mk_device("device-1", "First Name", "site-a");
        let duplicate = mk_device("device-1", "Second Name", "site-a");
        let unique = mk_device("device-2", "Unique Name", "site-b");

        let (devices, raw_json) = dedup_raw_devices_by_id(
            vec![first, duplicate, unique],
            vec![
                json!({"identification": {"id": "device-1", "hostname": "First Name"}}),
                json!({"identification": {"id": "device-1", "hostname": "Second Name"}}),
                json!({"identification": {"id": "device-2", "hostname": "Unique Name"}}),
            ],
        );

        assert_eq!(devices.len(), 2);
        assert_eq!(raw_json.len(), 2);
        assert_eq!(devices[0].identification.id, "device-1");
        assert_eq!(devices[0].get_name().as_deref(), Some("First Name"));
        assert_eq!(devices[1].identification.id, "device-2");
        assert_eq!(
            raw_json[0]["identification"]["hostname"].as_str(),
            Some("First Name")
        );
        assert_eq!(
            raw_json[1]["identification"]["hostname"].as_str(),
            Some("Unique Name")
        );
    }
}
