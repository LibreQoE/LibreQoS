use crate::errors::UispIntegrationError;
use crate::ethernet_advisory::{apply_ethernet_rate_cap, write_ethernet_advisories};
use crate::ip_ranges::IpRanges;
use crate::strategies::common::UispData;
use crate::strategies::full::shaped_devices_writer::ShapedDevice;
use lqos_config::{CircuitEthernetMetadata, Config};
use std::collections::{HashMap, HashSet};
use std::fs::write;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

fn build_ap_only_network_json(
    uisp_data: &UispData,
    mappings: &HashMap<String, Vec<String>>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut root = serde_json::Map::new();
    for ap_id in mappings.keys() {
        let Some(ap_device) = uisp_data.find_uisp_device_by_id(ap_id) else {
            warn!("Unable to find AP device for mapping key {ap_id}");
            continue;
        };
        let ap_name = uisp_data.device_display_name(ap_id);

        let mut ap_object = serde_json::Map::new();
        ap_object.insert("children".to_string(), serde_json::Map::new().into());
        ap_object.insert(
            "downloadBandwidthMbps".to_string(),
            serde_json::Value::Number(ap_device.download.into()),
        );
        ap_object.insert(
            "uploadBandwidthMbps".to_string(),
            serde_json::Value::Number(ap_device.upload.into()),
        );
        ap_object.insert("type".to_string(), "AP".to_string().into());
        ap_object.insert(
            "id".to_string(),
            format!("uisp:device:{}", ap_device.id).into(),
        );
        ap_object.insert("uisp_device".to_string(), ap_device.id.clone().into());

        root.insert(ap_name, ap_object.into());
    }
    root
}

/// Creates a network with only APs detected
/// from clients.
pub async fn build_ap_only_network(
    config: Arc<Config>,
    ip_ranges: IpRanges,
) -> Result<(), UispIntegrationError> {
    let uisp_data = UispData::fetch_uisp_data(config.clone(), ip_ranges).await?;

    // Find the clients
    let mappings = uisp_data.map_clients_to_aps();

    // Write network.json
    let network_path = Path::new(&config.lqos_directory).join("network.json");
    if network_path.exists() && !config.integration_common.always_overwrite_network_json {
        warn!(
            "Network.json exists, and always overwrite network json is not true - not writing network.json"
        );
    } else {
        let root = build_ap_only_network_json(&uisp_data, &mappings);
        let json = serde_json::to_string_pretty(&root).unwrap();
        write(network_path, json).map_err(|e| {
            error!("Unable to write network.json");
            error!("{e:?}");
            UispIntegrationError::WriteNetJson
        })?;
        info!("Written network.json");
    }

    // Write ShapedDevices.csv
    let file_path = Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let mut shaped_devices = Vec::new();
    let mut ethernet_advisories: Vec<CircuitEthernetMetadata> = Vec::new();
    let mut seen_pairs = HashSet::new();
    for (parent_id, client_ids) in mappings.iter() {
        let parent_name = uisp_data.device_display_name(parent_id);
        for client_id in client_ids {
            let site = uisp_data.sites.iter().find(|s| *client_id == s.id).unwrap();
            let devices = uisp_data
                .devices
                .iter()
                .filter(|d| d.site_id == *client_id)
                .collect::<Vec<_>>();
            let requested = if let Some((dl_min, dl_max, ul_min, ul_max)) =
                site.burst_rates(&config)
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
                &site.id,
                &site.name,
                devices.iter().copied(),
                requested.0,
                requested.2,
                requested.1,
                requested.3,
            );
            if let Some(advisory) = ethernet_decision.advisory.clone() {
                ethernet_advisories.push(advisory);
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
    write_ethernet_advisories(&config, &ethernet_advisories)?;
    info!("Wrote {} lines to ShapedDevices.csv", shaped_devices.len());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::build_ap_only_network_json;
    use crate::strategies::common::UispData;
    use crate::uisp_types::{UispDevice, UispSite};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn ap_only_network_json_resolves_ap_ids_to_names() {
        let uisp_data = UispData {
            sites_raw: vec![],
            devices_raw: vec![],
            data_links_raw: vec![],
            sites: vec![UispSite {
                id: "client-site".to_string(),
                name: "Client Site".to_string(),
                ..Default::default()
            }],
            devices: vec![UispDevice {
                id: "ap-1".to_string(),
                name: "Tower AP".to_string(),
                mac: "".to_string(),
                role: None,
                wireless_mode: None,
                site_id: "tower-site".to_string(),
                download: 500,
                upload: 400,
                ipv4: HashSet::new(),
                ipv6: HashSet::new(),
                negotiated_ethernet_mbps: None,
                negotiated_ethernet_interface: None,
            }],
        };
        let mappings = HashMap::from([("ap-1".to_string(), vec!["client-site".to_string()])]);

        let network_json = build_ap_only_network_json(&uisp_data, &mappings);

        assert!(network_json.get("Tower AP").is_some());
        let ap = network_json
            .get("Tower AP")
            .and_then(|value| value.as_object())
            .unwrap();
        assert_eq!(ap.get("uisp_device").and_then(|v| v.as_str()), Some("ap-1"));
    }
}
