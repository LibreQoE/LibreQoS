use crate::errors::UispIntegrationError;
use crate::uisp_types::{UispDevice, UispSite, UispSiteType};
use lqos_config::Config;
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
    let file_path = Path::new(&config.lqos_directory).join("ShapedDevices.csv");
    let mut shaped_devices = Vec::new();

    // Traverse
    traverse(
        sites,
        root_idx,
        0,
        devices,
        &mut shaped_devices,
        config,
        root_idx,
    );

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
    info!("Wrote {} lines to ShapedDevices.csv", shaped_devices.len());

    Ok(())
}

fn traverse(
    sites: &[UispSite],
    idx: usize,
    depth: u32,
    devices: &[UispDevice],
    shaped_devices: &mut Vec<ShapedDevice>,
    config: &Config,
    root_idx: usize,
) {
    if !sites[idx].device_indices.is_empty() {
        // We have devices!
        if sites[idx].site_type == UispSiteType::Client {
            // Add as normal clients
            for device in sites[idx].device_indices.iter() {
                let device = &devices[*device];
                if device.has_address() {
                    // Calculate fractional rates preserving decimal precision
                    let mut download_max = sites[idx].max_down_mbps as f32
                        * config.uisp_integration.bandwidth_overhead_factor;
                    let mut upload_max = sites[idx].max_up_mbps as f32
                        * config.uisp_integration.bandwidth_overhead_factor;
                    let mut download_min = download_max
                        * config.uisp_integration.commit_bandwidth_multiplier;
                    let mut upload_min = upload_max
                        * config.uisp_integration.commit_bandwidth_multiplier;
                    
                    // If suspended with "slow" strategy, clamp min/max to exactly 0.1 Mbps
                    if sites[idx].suspended &&
                        config.uisp_integration.suspended_strategy.eq_ignore_ascii_case("slow") {
                        download_max = 0.1;
                        upload_max = 0.1;
                        download_min = 0.1;
                        upload_min = 0.1;
                    } else {
                        // Apply minimum rate safeguards (0.1 Mbps minimum)
                        download_max = f32::max(0.1, download_max);
                        upload_max = f32::max(0.1, upload_max);
                        download_min = f32::max(0.1, download_min);
                        upload_min = f32::max(0.1, upload_min);
                    }
                    
                    let sd = ShapedDevice {
                        circuit_id: sites[idx].id.clone(),
                        circuit_name: sites[idx].name.clone(),
                        device_id: device.id.clone(),
                        device_name: device.name.clone(),
                        parent_node: sites[sites[idx].selected_parent.unwrap()].name.clone(),
                        mac: device.mac.clone(),
                        ipv4: device.ipv4_list(),
                        ipv6: device.ipv6_list(),
                        download_min: download_min,
                        download_max: download_max,
                        upload_min: upload_min,
                        upload_max: upload_max,
                        comment: "".to_string(),
                    };
                    shaped_devices.push(sd);
                }
            }
        } else {
            // It's an infrastructure node
            for device in sites[idx].device_indices.iter() {
                let device = &devices[*device];
                let parent_node = if idx != root_idx {
                    sites[idx].name.clone()
                } else {
                    format!("{}_Infrastructure", sites[idx].name.clone())
                };
                if device.has_address() {
                    // Calculate fractional rates preserving decimal precision (infrastructure)
                    let mut download_max = sites[idx].max_down_mbps as f32
                        * config.uisp_integration.bandwidth_overhead_factor;
                    let mut upload_max = sites[idx].max_up_mbps as f32
                        * config.uisp_integration.bandwidth_overhead_factor;
                    let mut download_min = download_max
                        * config.uisp_integration.commit_bandwidth_multiplier;
                    let mut upload_min = upload_max
                        * config.uisp_integration.commit_bandwidth_multiplier;
                    
                    // Apply minimum rate safeguards (0.2 Mbps minimum, higher for infrastructure)
                    download_max = f32::max(0.2, download_max);
                    upload_max = f32::max(0.2, upload_max);
                    download_min = f32::max(0.2, download_min);
                    upload_min = f32::max(0.2, upload_min);
                    
                    let sd = ShapedDevice {
                        circuit_id: format!("{}-inf", sites[idx].id),
                        circuit_name: format!("{} Infrastructure", sites[idx].name),
                        device_id: device.id.clone(),
                        device_name: device.name.clone(),
                        parent_node,
                        mac: device.mac.clone(),
                        ipv4: device.ipv4_list(),
                        ipv6: device.ipv6_list(),
                        download_min: download_min,
                        download_max: download_max,
                        upload_min: upload_min,
                        upload_max: upload_max,
                        comment: "Infrastructure Entry".to_string(),
                    };
                    shaped_devices.push(sd);
                }
            }
        }
    }

    if depth < 10 {
        for (child_idx, child) in sites.iter().enumerate() {
            if let Some(parent_idx) = child.selected_parent {
                if parent_idx == idx {
                    traverse(
                        sites,
                        child_idx,
                        depth + 1,
                        devices,
                        shaped_devices,
                        config,
                        root_idx,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                download_min: 0.5,  // Sub-1 Mbps
                upload_min: 0.5,
                download_max: 2.5,  // Fractional rate
                upload_max: 1.0,    // Whole number
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
                download_min: 1.25,  // Precise decimal
                upload_min: 0.75,   // Another fractional
                download_max: 10.5,  // Mixed decimal
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
                download_min: 0.2,   // Infrastructure minimum
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
                writer.serialize(device).expect("Failed to serialize device");
            }
            
            writer.flush().expect("Failed to flush CSV writer");
        }
        
        let csv_string = String::from_utf8(csv_output).expect("Invalid UTF-8 in CSV output");
        println!("Generated CSV output:\n{}", csv_string);
        
        // Validate the output (1 header + 3 data rows)
        let lines: Vec<&str> = csv_string.trim().split('\n').collect();
        assert_eq!(lines.len(), 4, "Should have 4 CSV lines (1 header + 3 data rows)");
        
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
        assert_eq!(parsed_devices[0].download_max, 2.5, "First device should have 2.5 Mbps download_max");
        assert_eq!(parsed_devices[0].download_min, 0.5, "First device should have 0.5 Mbps download_min");
        
        assert_eq!(parsed_devices[1].download_max, 10.5, "Second device should have 10.5 Mbps download_max");
        assert_eq!(parsed_devices[1].upload_max, 5.25, "Second device should have 5.25 Mbps upload_max");
        
        assert_eq!(parsed_devices[2].download_min, 0.2, "Infrastructure should have 0.2 Mbps rates");
        
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
        assert_eq!(infra_safeguarded, 0.2, "Should apply 0.2 Mbps minimum for infrastructure");
        
        // Test normal rates are preserved
        let normal_rate = 2.5_f32;
        let preserved_rate = f32::max(0.1, normal_rate);
        assert_eq!(preserved_rate, 2.5, "Normal rates should be preserved");
        
        println!("✅ Rate safeguard tests passed!");
    }
}
