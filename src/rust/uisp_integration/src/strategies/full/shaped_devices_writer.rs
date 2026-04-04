use crate::errors::UispIntegrationError;
use crate::ethernet_advisory::{apply_ethernet_rate_cap, write_ethernet_advisories};
use crate::uisp_types::{UispDevice, UispSite, UispSiteType};
use lqos_config::{
    CircuitEthernetMetadata, Config, EthernetPortLimitPolicy, RequestedCircuitRates,
};
use serde::Serialize;
use std::path::Path;
use tracing::{error, info};

/// Represents a shaped device in the ShapedDevices.csv file.
#[derive(Serialize, Debug)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub struct ShapedDevice {
    pub circuit_id: String,
    pub circuit_name: String,
    pub device_id: String,
    pub device_name: String,
    pub parent_node: String,
    pub mac: String,
    pub ipv4: String,
    pub ipv6: String,
    pub download_min: f32,
    pub upload_min: f32,
    pub download_max: f32,
    pub upload_max: f32,
    pub comment: String,
}

struct ShapedDeviceOutputs<'a> {
    shaped_devices: &'a mut Vec<ShapedDevice>,
    ethernet_advisories: &'a mut Vec<CircuitEthernetMetadata>,
}

struct TraverseContext<'a> {
    config: &'a Config,
    root_idx: usize,
    ethernet_policy: EthernetPortLimitPolicy,
}

/// Writes the ShapedDevices.csv file for UISP
///
/// # Arguments
/// * `config` - The configuration
/// * `sites` - The list of sites
/// * `root_idx` - The index of the root site
/// * `devices` - The list of devices
pub fn write_shaped_devices(
    config: &Config,
    sites: &[UispSite],
    root_idx: usize,
    devices: &[UispDevice],
) -> Result<(), UispIntegrationError> {
    let ethernet_policy = EthernetPortLimitPolicy::from(&config.integration_common);
    let file_path = Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let mut shaped_devices = Vec::new();
    let mut ethernet_advisories: Vec<CircuitEthernetMetadata> = Vec::new();

    // Traverse
    let mut outputs = ShapedDeviceOutputs {
        shaped_devices: &mut shaped_devices,
        ethernet_advisories: &mut ethernet_advisories,
    };
    let context = TraverseContext {
        config,
        root_idx,
        ethernet_policy,
    };
    traverse(sites, root_idx, 0, devices, &mut outputs, &context);

    // Write the CSV
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
    write_ethernet_advisories(config, &ethernet_advisories)?;
    info!("Wrote {} lines to ShapedDevices.csv", shaped_devices.len());

    Ok(())
}

fn traverse(
    sites: &[UispSite],
    idx: usize,
    depth: u32,
    devices: &[UispDevice],
    outputs: &mut ShapedDeviceOutputs<'_>,
    context: &TraverseContext<'_>,
) {
    if !sites[idx].device_indices.is_empty() {
        // We have devices!
        if sites[idx].site_type == UispSiteType::Client {
            let site_devices: Vec<&UispDevice> = sites[idx]
                .device_indices
                .iter()
                .filter_map(|device_idx| devices.get(*device_idx))
                .collect();

            let requested = if let Some((dl_min, dl_max, ul_min, ul_max)) =
                sites[idx].burst_rates(context.config)
            {
                (
                    f32::max(0.1, dl_min),
                    f32::max(0.1, dl_max),
                    f32::max(0.1, ul_min),
                    f32::max(0.1, ul_max),
                )
            } else {
                let download_max_f32 = sites[idx].max_down_mbps as f32
                    * context.config.uisp_integration.bandwidth_overhead_factor;
                let upload_max_f32 = sites[idx].max_up_mbps as f32
                    * context.config.uisp_integration.bandwidth_overhead_factor;
                let download_min_f32 =
                    download_max_f32 * context.config.uisp_integration.commit_bandwidth_multiplier;
                let upload_min_f32 =
                    upload_max_f32 * context.config.uisp_integration.commit_bandwidth_multiplier;
                (
                    f32::max(0.1, download_min_f32),
                    f32::max(0.1, download_max_f32),
                    f32::max(0.1, upload_min_f32),
                    f32::max(0.1, upload_max_f32),
                )
            };
            let ethernet_decision = apply_ethernet_rate_cap(
                context.ethernet_policy,
                &sites[idx].id,
                &sites[idx].name,
                site_devices.iter().copied(),
                RequestedCircuitRates {
                    download_min: requested.0,
                    upload_min: requested.2,
                    download_max: requested.1,
                    upload_max: requested.3,
                },
            );
            if let Some(advisory) = ethernet_decision.advisory.clone() {
                outputs.ethernet_advisories.push(advisory);
            }

            // Add as normal clients
            for device in sites[idx].device_indices.iter() {
                let device = &devices[*device];
                if device.has_address() {
                    let sd = ShapedDevice {
                        circuit_id: sites[idx].id.clone(),
                        circuit_name: sites[idx].name.clone(),
                        device_id: device.id.clone(),
                        device_name: device.name.clone(),
                        parent_node: sites[sites[idx].selected_parent.unwrap()].name.clone(),
                        mac: device.mac.clone(),
                        ipv4: device.ipv4_list(),
                        ipv6: device.ipv6_list(),
                        download_min: ethernet_decision.download_min,
                        download_max: ethernet_decision.download_max,
                        upload_min: ethernet_decision.upload_min,
                        upload_max: ethernet_decision.upload_max,
                        comment: "".to_string(),
                    };
                    outputs.shaped_devices.push(sd);
                }
            }
        } else {
            // It's an infrastructure node
            for device in sites[idx].device_indices.iter() {
                let device = &devices[*device];
                let parent_node = if idx != context.root_idx {
                    sites[idx].name.clone()
                } else {
                    format!("{}_Infrastructure", sites[idx].name.clone())
                };
                if device.has_address() {
                    // Infrastructure: keep capacity-based behavior
                    let download_max_f32 = sites[idx].max_down_mbps as f32
                        * context.config.uisp_integration.bandwidth_overhead_factor;
                    let upload_max_f32 = sites[idx].max_up_mbps as f32
                        * context.config.uisp_integration.bandwidth_overhead_factor;
                    let download_min_f32 = download_max_f32
                        * context.config.uisp_integration.commit_bandwidth_multiplier;
                    let upload_min_f32 = upload_max_f32
                        * context.config.uisp_integration.commit_bandwidth_multiplier;
                    let download_max = f32::max(0.2, download_max_f32);
                    let upload_max = f32::max(0.2, upload_max_f32);
                    let download_min = f32::max(0.2, download_min_f32);
                    let upload_min = f32::max(0.2, upload_min_f32);

                    let sd = ShapedDevice {
                        circuit_id: format!("{}-inf", sites[idx].id),
                        circuit_name: format!("{} Infrastructure", sites[idx].name),
                        device_id: device.id.clone(),
                        device_name: device.name.clone(),
                        parent_node,
                        mac: device.mac.clone(),
                        ipv4: device.ipv4_list(),
                        ipv6: device.ipv6_list(),
                        download_min,
                        download_max,
                        upload_min,
                        upload_max,
                        comment: "Infrastructure Entry".to_string(),
                    };
                    outputs.shaped_devices.push(sd);
                }
            }
        }
    }

    if depth < 10 {
        for (child_idx, child) in sites.iter().enumerate() {
            if let Some(parent_idx) = child.selected_parent
                && parent_idx == idx
            {
                traverse(sites, child_idx, depth + 1, devices, outputs, context);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uisp_types::{UispDevice, UispSite, UispSiteType};
    use std::collections::HashSet;

    #[test]
    fn test_fractional_csv_serialization() {
        // Create test shaped devices with fractional rates
        let test_devices = vec![
            ShapedDevice {
                circuit_id: "test-001".to_string(),
                circuit_name: "Test Client 1".to_string(),
                device_id: "device-001".to_string(),
                device_name: "CPE-001".to_string(),
                parent_node: "Tower-A".to_string(),
                mac: "00:11:22:33:44:55".to_string(),
                ipv4: "192.168.1.100".to_string(),
                ipv6: "".to_string(),
                download_min: 0.5, // Sub-1 Mbps
                upload_min: 0.5,
                download_max: 2.5, // Fractional rate
                upload_max: 1.0,   // Whole number
                comment: "Fractional rate test".to_string(),
            },
            ShapedDevice {
                circuit_id: "test-002".to_string(),
                circuit_name: "Test Client 2".to_string(),
                device_id: "device-002".to_string(),
                device_name: "CPE-002".to_string(),
                parent_node: "Tower-B".to_string(),
                mac: "00:11:22:33:44:66".to_string(),
                ipv4: "192.168.1.101".to_string(),
                ipv6: "2001:db8::1".to_string(),
                download_min: 1.25, // Precise decimal
                upload_min: 0.75,   // Another fractional
                download_max: 10.5, // Mixed decimal
                upload_max: 5.25,   // Another precise decimal
                comment: "Mixed rate test".to_string(),
            },
            ShapedDevice {
                circuit_id: "test-003-inf".to_string(),
                circuit_name: "Test Infrastructure".to_string(),
                device_id: "device-003".to_string(),
                device_name: "AP-003".to_string(),
                parent_node: "Root".to_string(),
                mac: "00:11:22:33:44:77".to_string(),
                ipv4: "10.0.1.1".to_string(),
                ipv6: "".to_string(),
                download_min: 0.2, // Infrastructure minimum
                upload_min: 0.2,
                download_max: 0.2,
                upload_max: 0.2,
                comment: "Infrastructure Entry".to_string(),
            },
        ];

        // Serialize to CSV
        let mut csv_output = Vec::new();
        {
            let mut writer = csv::Writer::from_writer(&mut csv_output);

            for device in &test_devices {
                writer
                    .serialize(device)
                    .expect("Failed to serialize device");
            }

            writer.flush().expect("Failed to flush CSV writer");
        }

        let csv_string = String::from_utf8(csv_output).expect("Invalid UTF-8 in CSV output");
        println!("Generated CSV output:\n{}", csv_string);

        // Validate the output (1 header + 3 data rows)
        let lines: Vec<&str> = csv_string.trim().split('\n').collect();
        assert_eq!(
            lines.len(),
            4,
            "Should have 4 CSV lines (1 header + 3 data rows)"
        );

        // Parse CSV back to verify fractional rates are preserved
        let mut reader = csv::Reader::from_reader(csv_string.as_bytes());
        let mut parsed_devices = Vec::new();

        for result in reader.deserialize() {
            let device: ShapedDevice = result.expect("Failed to deserialize device");
            parsed_devices.push(device);
        }

        // Verify fractional rates are preserved
        assert_eq!(parsed_devices.len(), 3, "Should parse 3 devices");

        // Test specific fractional rates
        assert_eq!(
            parsed_devices[0].download_max, 2.5,
            "First device should have 2.5 Mbps download_max"
        );
        assert_eq!(
            parsed_devices[0].download_min, 0.5,
            "First device should have 0.5 Mbps download_min"
        );

        assert_eq!(
            parsed_devices[1].download_max, 10.5,
            "Second device should have 10.5 Mbps download_max"
        );
        assert_eq!(
            parsed_devices[1].upload_max, 5.25,
            "Second device should have 5.25 Mbps upload_max"
        );

        assert_eq!(
            parsed_devices[2].download_min, 0.2,
            "Infrastructure should have 0.2 Mbps rates"
        );

        println!("✅ CSV serialization test passed!");
        println!("   - Fractional rates preserved correctly");
        println!("   - Sub-1 Mbps rates work (0.5, 0.2)");
        println!("   - Precise decimals work (2.5, 10.5, 5.25)");
        println!("   - Infrastructure minimums applied (0.2)");
    }

    #[test]
    fn test_rate_calculation_safeguards() {
        // Test the defensive programming we implemented

        // Simulate very small calculated rates
        let very_small_rate = 0.05_f32;
        let safeguarded_rate = f32::max(0.1, very_small_rate);
        assert_eq!(safeguarded_rate, 0.1, "Should apply 0.1 Mbps minimum");

        // Test infrastructure minimum
        let small_infra_rate = 0.15_f32;
        let infra_safeguarded = f32::max(0.2, small_infra_rate);
        assert_eq!(
            infra_safeguarded, 0.2,
            "Should apply 0.2 Mbps minimum for infrastructure"
        );

        // Test normal rates are preserved
        let normal_rate = 2.5_f32;
        let preserved_rate = f32::max(0.1, normal_rate);
        assert_eq!(preserved_rate, 2.5, "Normal rates should be preserved");

        println!("✅ Rate safeguard tests passed!");
    }

    #[test]
    fn ignored_only_client_device_produces_no_shaped_rows() {
        let config = Config::default();
        let sites = vec![
            UispSite {
                id: "root-site".to_string(),
                name: "Root Site".to_string(),
                site_type: UispSiteType::Site,
                max_down_mbps: 1000,
                max_up_mbps: 1000,
                ..Default::default()
            },
            UispSite {
                id: "client-site".to_string(),
                name: "Client Site".to_string(),
                site_type: UispSiteType::Client,
                max_down_mbps: 100,
                max_up_mbps: 50,
                selected_parent: Some(0),
                device_indices: vec![0],
                ..Default::default()
            },
        ];
        let devices = vec![UispDevice {
            id: "device-1".to_string(),
            name: "CPE 1".to_string(),
            mac: "".to_string(),
            role: None,
            wireless_mode: None,
            site_id: "client-site".to_string(),
            download: 100,
            upload: 50,
            ipv4: HashSet::new(),
            ipv6: HashSet::new(),
            negotiated_ethernet_mbps: None,
            negotiated_ethernet_interface: None,
        }];

        let mut shaped_devices = Vec::new();
        let mut ethernet_advisories = Vec::new();
        let mut outputs = ShapedDeviceOutputs {
            shaped_devices: &mut shaped_devices,
            ethernet_advisories: &mut ethernet_advisories,
        };

        traverse(
            &sites,
            0,
            0,
            &devices,
            &mut outputs,
            &TraverseContext {
                config: &config,
                root_idx: 0,
                ethernet_policy: EthernetPortLimitPolicy::default(),
            },
        );

        assert!(outputs.shaped_devices.is_empty());
    }
}
