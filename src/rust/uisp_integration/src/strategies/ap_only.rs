use crate::errors::UispIntegrationError;
use crate::ethernet_advisory::{apply_ethernet_rate_cap, write_ethernet_advisories};
use crate::ip_ranges::IpRanges;
use crate::strategies::common::UispData;
use crate::strategies::full::shaped_devices_writer::{ShapedDevice, write_circuit_anchors};
use lqos_config::{
    CircuitAnchor, CircuitEthernetMetadata, Config, EthernetPortLimitPolicy, RequestedCircuitRates,
};
use std::collections::{HashMap, HashSet};
use std::fs::write;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExportedApNode {
    node_id: String,
    node_name: String,
    uisp_device_id: String,
    download_mbps: u64,
    upload_mbps: u64,
}

fn short_device_id(device_id: &str) -> &str {
    let trimmed = device_id.trim();
    let len = trimmed.len().min(8);
    &trimmed[..len]
}

fn disambiguate_exported_ap_names(exported: &mut HashMap<String, ExportedApNode>) {
    let mut ids_by_name = HashMap::<String, Vec<String>>::new();
    for node in exported.values() {
        ids_by_name
            .entry(node.node_name.clone())
            .or_default()
            .push(node.uisp_device_id.clone());
    }

    let mut used_names = HashSet::<String>::new();
    for ids in ids_by_name.values_mut() {
        ids.sort();
        for device_id in ids.iter() {
            let Some(node) = exported.get_mut(device_id) else {
                continue;
            };
            let base_name = node.node_name.trim();
            if ids.len() == 1 && used_names.insert(base_name.to_string()) {
                continue;
            }

            let mut candidate = format!("{base_name} ({})", short_device_id(device_id));
            if used_names.contains(&candidate) {
                candidate = format!("{base_name} ({device_id})");
            }
            used_names.insert(candidate.clone());
            node.node_name = candidate;
        }
    }
}

fn build_exported_ap_nodes(
    uisp_data: &UispData,
    mappings: &HashMap<String, Vec<String>>,
) -> HashMap<String, ExportedApNode> {
    let mut exported = HashMap::new();
    for ap_id in mappings.keys() {
        let Some(ap_device) = uisp_data.find_uisp_device_by_id(ap_id) else {
            warn!("Unable to export AP-only parent mapping key {ap_id} into network.json");
            continue;
        };
        exported.insert(
            ap_id.clone(),
            ExportedApNode {
                node_id: format!("uisp:device:{}", ap_device.id),
                node_name: uisp_data.device_display_name(ap_id),
                uisp_device_id: ap_device.id.clone(),
                download_mbps: ap_device.download,
                upload_mbps: ap_device.upload,
            },
        );
    }
    disambiguate_exported_ap_names(&mut exported);
    exported
}

fn build_ap_only_network_json(
    exported_aps: &HashMap<String, ExportedApNode>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut root = serde_json::Map::new();
    let mut exported_nodes = exported_aps.values().collect::<Vec<_>>();
    exported_nodes.sort_unstable_by(|left, right| left.node_name.cmp(&right.node_name));
    for exported in exported_nodes {
        let mut ap_object = serde_json::Map::new();
        ap_object.insert("children".to_string(), serde_json::Map::new().into());
        ap_object.insert(
            "downloadBandwidthMbps".to_string(),
            serde_json::Value::Number(exported.download_mbps.into()),
        );
        ap_object.insert(
            "uploadBandwidthMbps".to_string(),
            serde_json::Value::Number(exported.upload_mbps.into()),
        );
        ap_object.insert("type".to_string(), "AP".to_string().into());
        ap_object.insert("id".to_string(), exported.node_id.clone().into());
        ap_object.insert(
            "uisp_device".to_string(),
            exported.uisp_device_id.clone().into(),
        );

        root.insert(exported.node_name.clone(), ap_object.into());
    }
    root
}

fn build_ap_only_shaping_outputs(
    config: &Config,
    ethernet_policy: EthernetPortLimitPolicy,
    uisp_data: &UispData,
    mappings: &HashMap<String, Vec<String>>,
    exported_aps: &HashMap<String, ExportedApNode>,
) -> (
    Vec<ShapedDevice>,
    Vec<CircuitAnchor>,
    Vec<CircuitEthernetMetadata>,
) {
    let mut shaped_devices = Vec::new();
    let mut circuit_anchors = Vec::<CircuitAnchor>::new();
    let mut ethernet_advisories: Vec<CircuitEthernetMetadata> = Vec::new();
    let mut seen_pairs = HashSet::new();
    let mut seen_circuits = HashSet::new();

    for (parent_id, client_ids) in mappings {
        let exported_parent = exported_aps.get(parent_id);
        let parent_name = exported_parent
            .map(|parent| parent.node_name.clone())
            .unwrap_or_default();
        let parent_node_id = exported_parent
            .map(|parent| parent.node_id.clone())
            .unwrap_or_default();

        for client_id in client_ids {
            let site = uisp_data.sites.iter().find(|s| *client_id == s.id).unwrap();
            let devices = uisp_data
                .devices
                .iter()
                .filter(|d| d.site_id == *client_id)
                .collect::<Vec<_>>();
            let requested = if let Some((dl_min, dl_max, ul_min, ul_max)) = site.burst_rates(config)
            {
                (
                    f32::max(0.1, dl_min),
                    f32::max(0.1, dl_max),
                    f32::max(0.1, ul_min),
                    f32::max(0.1, ul_max),
                )
            } else if site.suspended && config.uisp_integration.suspended_strategy == "slow" {
                (0.1, 0.1, 0.1, 0.1)
            } else {
                (
                    f32::max(
                        0.1,
                        site.max_down_mbps as f32
                            * config.uisp_integration.commit_bandwidth_multiplier,
                    ),
                    f32::max(
                        0.1,
                        site.max_down_mbps as f32
                            * config.uisp_integration.bandwidth_overhead_factor,
                    ),
                    f32::max(
                        0.1,
                        site.max_up_mbps as f32
                            * config.uisp_integration.commit_bandwidth_multiplier,
                    ),
                    f32::max(
                        0.1,
                        site.max_up_mbps as f32 * config.uisp_integration.bandwidth_overhead_factor,
                    ),
                )
            };
            let ethernet_decision = apply_ethernet_rate_cap(
                ethernet_policy,
                &site.id,
                &site.name,
                devices.iter().copied(),
                RequestedCircuitRates {
                    download_min: requested.0,
                    upload_min: requested.2,
                    download_max: requested.1,
                    upload_max: requested.3,
                },
            );
            if let Some(advisory) = ethernet_decision.advisory.clone() {
                ethernet_advisories.push(advisory);
            }
            if devices.iter().any(|device| device.has_address())
                && seen_circuits.insert(site.id.clone())
                && !parent_node_id.is_empty()
            {
                circuit_anchors.push(CircuitAnchor {
                    circuit_id: site.id.clone(),
                    circuit_name: Some(site.name.clone()),
                    anchor_node_id: parent_node_id.clone(),
                    anchor_node_name: Some(parent_name.clone()),
                });
            }
            for device in devices.iter().filter(|d| d.has_address()) {
                let key = (site.id.clone(), device.id.clone());
                if !seen_pairs.insert(key) {
                    continue;
                }

                let sd = ShapedDevice {
                    circuit_id: site.id.clone(),
                    circuit_name: site.name.clone(),
                    device_id: device.id.clone(),
                    device_name: device.name.clone(),
                    parent_node: parent_name.clone(),
                    parent_node_id: parent_node_id.clone(),
                    anchor_node_id: String::new(),
                    mac: device.mac.clone(),
                    ipv4: device.ipv4_list(),
                    ipv6: device.ipv6_list(),
                    download_min: ethernet_decision.download_min,
                    upload_min: ethernet_decision.upload_min,
                    download_max: ethernet_decision.download_max,
                    upload_max: ethernet_decision.upload_max,
                    comment: "".to_string(),
                };
                shaped_devices.push(sd);
            }
        }
    }

    (shaped_devices, circuit_anchors, ethernet_advisories)
}

/// Creates a network with only APs detected
/// from clients.
pub async fn build_ap_only_network(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    let uisp_data = UispData::fetch_uisp_data(config.clone(), ip_ranges).await?;
    let ethernet_policy = EthernetPortLimitPolicy::from(&config.integration_common);

    // Find the clients
    let mappings = uisp_data.map_clients_to_aps();
    let exported_aps = build_exported_ap_nodes(&uisp_data, &mappings);

    // Write network.json
    let network_path = Path::new(&config.lqos_directory).join("network.json");
    let root = build_ap_only_network_json(&exported_aps);
    let json = serde_json::to_string_pretty(&root).unwrap();
    write(network_path, json).map_err(|e| {
        error!("Unable to write network.json");
        error!("{e:?}");
        UispIntegrationError::WriteNetJson
    })?;
    info!("Written network.json");

    // Write ShapedDevices.csv
    let file_path = Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let (shaped_devices, circuit_anchors, ethernet_advisories) = build_ap_only_shaping_outputs(
        &config,
        ethernet_policy,
        &uisp_data,
        &mappings,
        &exported_aps,
    );
    let mut writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_path(file_path)
        .unwrap();

    for d in shaped_devices.iter() {
        writer.serialize(d).unwrap();
    }
    writer.flush().map_err(|e| {
        error!("Unable to flush CSV file");
        error!("{e:?}");
        UispIntegrationError::CsvError
    })?;
    write_circuit_anchors(&config, "uisp/ap_only", &circuit_anchors)?;
    write_ethernet_advisories(&config, &ethernet_advisories)?;
    info!("Wrote {} lines to ShapedDevices.csv", shaped_devices.len());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_ap_only_network_json, build_ap_only_shaping_outputs, build_exported_ap_nodes,
    };
    use crate::strategies::common::UispData;
    use crate::uisp_types::{UispAttachmentRateSource, UispDevice, UispSite, UispSiteType};
    use lqos_config::{Config, EthernetPortLimitPolicy};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn ap_only_network_json_resolves_ap_ids_to_names() {
        let uisp_data = UispData::from_parts(
            vec![],
            vec![],
            vec![],
            vec![UispSite {
                id: "client-site".to_string(),
                name: "Client Site".to_string(),
                ..Default::default()
            }],
            vec![UispDevice {
                id: "ap-1".to_string(),
                name: "Tower AP".to_string(),
                mac: "".to_string(),
                role: None,
                wireless_mode: None,
                site_id: "tower-site".to_string(),
                raw_download: 500,
                raw_upload: 400,
                download: 500,
                upload: 400,
                ipv4: HashSet::new(),
                ipv6: HashSet::new(),
                probe_ipv4: HashSet::new(),
                probe_ipv6: HashSet::new(),
                negotiated_ethernet_mbps: None,
                negotiated_ethernet_interface: None,
                transport_cap_mbps: None,
                transport_cap_reason: None,
                attachment_rate_source: UispAttachmentRateSource::Static,
            }],
        );
        let mappings = HashMap::from([("ap-1".to_string(), vec!["client-site".to_string()])]);
        let exported_aps = build_exported_ap_nodes(&uisp_data, &mappings);

        let network_json = build_ap_only_network_json(&exported_aps);

        assert!(network_json.get("Tower AP").is_some());
        let ap = network_json
            .get("Tower AP")
            .and_then(|value| value.as_object())
            .unwrap();
        assert_eq!(ap.get("uisp_device").and_then(|v| v.as_str()), Some("ap-1"));
    }

    #[test]
    fn ap_only_shaping_outputs_skip_non_exported_parent_mappings() {
        let uisp_data = UispData::from_parts(
            vec![],
            vec![],
            vec![],
            vec![
                UispSite {
                    id: "client-valid".to_string(),
                    name: "Client Valid".to_string(),
                    site_type: UispSiteType::Client,
                    max_down_mbps: 100,
                    max_up_mbps: 20,
                    ..Default::default()
                },
                UispSite {
                    id: "client-orphan".to_string(),
                    name: "Client Orphan".to_string(),
                    site_type: UispSiteType::Client,
                    max_down_mbps: 80,
                    max_up_mbps: 10,
                    ..Default::default()
                },
            ],
            vec![
                UispDevice {
                    id: "ap-1".to_string(),
                    name: "Tower AP".to_string(),
                    mac: "".to_string(),
                    role: None,
                    wireless_mode: None,
                    site_id: "tower-site".to_string(),
                    raw_download: 500,
                    raw_upload: 400,
                    download: 500,
                    upload: 400,
                    ipv4: HashSet::new(),
                    ipv6: HashSet::new(),
                    probe_ipv4: HashSet::new(),
                    probe_ipv6: HashSet::new(),
                    negotiated_ethernet_mbps: None,
                    negotiated_ethernet_interface: None,
                    transport_cap_mbps: None,
                    transport_cap_reason: None,
                    attachment_rate_source: UispAttachmentRateSource::Static,
                },
                UispDevice {
                    id: "device-valid".to_string(),
                    name: "Valid Device".to_string(),
                    mac: "aa:bb:cc:dd:ee:01".to_string(),
                    role: None,
                    wireless_mode: None,
                    site_id: "client-valid".to_string(),
                    raw_download: 100,
                    raw_upload: 20,
                    download: 100,
                    upload: 20,
                    ipv4: HashSet::from(["192.0.2.10/32".to_string()]),
                    ipv6: HashSet::new(),
                    probe_ipv4: HashSet::new(),
                    probe_ipv6: HashSet::new(),
                    negotiated_ethernet_mbps: None,
                    negotiated_ethernet_interface: None,
                    transport_cap_mbps: None,
                    transport_cap_reason: None,
                    attachment_rate_source: UispAttachmentRateSource::Static,
                },
                UispDevice {
                    id: "device-orphan".to_string(),
                    name: "Orphan Device".to_string(),
                    mac: "aa:bb:cc:dd:ee:02".to_string(),
                    role: None,
                    wireless_mode: None,
                    site_id: "client-orphan".to_string(),
                    raw_download: 80,
                    raw_upload: 10,
                    download: 80,
                    upload: 10,
                    ipv4: HashSet::from(["192.0.2.20/32".to_string()]),
                    ipv6: HashSet::new(),
                    probe_ipv4: HashSet::new(),
                    probe_ipv6: HashSet::new(),
                    negotiated_ethernet_mbps: None,
                    negotiated_ethernet_interface: None,
                    transport_cap_mbps: None,
                    transport_cap_reason: None,
                    attachment_rate_source: UispAttachmentRateSource::Static,
                },
            ],
        );
        let mappings = HashMap::from([
            ("ap-1".to_string(), vec!["client-valid".to_string()]),
            ("Orphans".to_string(), vec!["client-orphan".to_string()]),
        ]);
        let exported_aps = build_exported_ap_nodes(&uisp_data, &mappings);

        let (shaped_devices, circuit_anchors, _ethernet_advisories) = build_ap_only_shaping_outputs(
            &Config::default(),
            EthernetPortLimitPolicy::default(),
            &uisp_data,
            &mappings,
            &exported_aps,
        );

        assert_eq!(circuit_anchors.len(), 1);
        assert_eq!(circuit_anchors[0].circuit_id, "client-valid");
        assert_eq!(circuit_anchors[0].anchor_node_id, "uisp:device:ap-1");

        let valid_device = shaped_devices
            .iter()
            .find(|device| device.device_id == "device-valid")
            .expect("expected valid device");
        assert_eq!(valid_device.parent_node, "Tower AP");
        assert_eq!(valid_device.parent_node_id, "uisp:device:ap-1");

        let orphan_device = shaped_devices
            .iter()
            .find(|device| device.device_id == "device-orphan")
            .expect("expected orphan device");
        assert!(orphan_device.parent_node.is_empty());
        assert!(orphan_device.parent_node_id.is_empty());
    }

    #[test]
    fn ap_only_exported_ap_names_are_disambiguated_when_display_names_collide() {
        let uisp_data = UispData::from_parts(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![
                UispDevice {
                    id: "ap-dup-1-12345678".to_string(),
                    name: "Shared AP".to_string(),
                    mac: "".to_string(),
                    role: None,
                    wireless_mode: None,
                    site_id: "site-a".to_string(),
                    raw_download: 100,
                    raw_upload: 100,
                    download: 100,
                    upload: 100,
                    ipv4: HashSet::new(),
                    ipv6: HashSet::new(),
                    probe_ipv4: HashSet::new(),
                    probe_ipv6: HashSet::new(),
                    negotiated_ethernet_mbps: None,
                    negotiated_ethernet_interface: None,
                    transport_cap_mbps: None,
                    transport_cap_reason: None,
                    attachment_rate_source: UispAttachmentRateSource::Static,
                },
                UispDevice {
                    id: "ap-dup-2-87654321".to_string(),
                    name: "Shared AP".to_string(),
                    mac: "".to_string(),
                    role: None,
                    wireless_mode: None,
                    site_id: "site-b".to_string(),
                    raw_download: 90,
                    raw_upload: 90,
                    download: 90,
                    upload: 90,
                    ipv4: HashSet::new(),
                    ipv6: HashSet::new(),
                    probe_ipv4: HashSet::new(),
                    probe_ipv6: HashSet::new(),
                    negotiated_ethernet_mbps: None,
                    negotiated_ethernet_interface: None,
                    transport_cap_mbps: None,
                    transport_cap_reason: None,
                    attachment_rate_source: UispAttachmentRateSource::Static,
                },
            ],
        );
        let mappings = HashMap::from([
            (
                "ap-dup-1-12345678".to_string(),
                vec!["client-a".to_string()],
            ),
            (
                "ap-dup-2-87654321".to_string(),
                vec!["client-b".to_string()],
            ),
        ]);

        let exported_aps = build_exported_ap_nodes(&uisp_data, &mappings);
        let network_json = build_ap_only_network_json(&exported_aps);

        assert_eq!(network_json.len(), 2);
        assert!(
            network_json
                .keys()
                .all(|name| name.starts_with("Shared AP ("))
        );
        assert!(
            network_json.contains_key("Shared AP (ap-dup-2)")
                || network_json.contains_key("Shared AP (ap-dup-2-87654321)")
        );
        assert!(
            network_json.contains_key("Shared AP (ap-dup-1)")
                || network_json.contains_key("Shared AP (ap-dup-1-12345678)")
        );
    }
}
