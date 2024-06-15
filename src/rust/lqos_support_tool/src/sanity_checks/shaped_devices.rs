use std::path::Path;
use lqos_config::load_config;
use crate::sanity_checks::SanityCheck;

pub fn shaped_devices_exists(results: &mut Vec<SanityCheck>) {
    if let Ok(cfg) = load_config() {
        let path = Path::new(&cfg.lqos_directory).join("ShapedDevices.csv");
        if path.exists() {
            results.push(SanityCheck{
                name: "ShapedDevices.csv exists".to_string(),
                success: true,
                comments: "".to_string(),
            });
        } else {
            results.push(SanityCheck{
                name: "ShapedDevices.csv exists".to_string(),
                success: false,
                comments: format!("File not found at {:?}", path),
            });
        }
    }
}

pub fn can_we_read_shaped_devices(results: &mut Vec<SanityCheck>) {
    match lqos_config::ConfigShapedDevices::load() {
        Ok(sd) => {
            results.push(SanityCheck{
                name: "ShapedDevices.csv Loads?".to_string(),
                success: true,
                comments: format!("{} Devices Found", sd.devices.len()),
            });
        }
        Err(e) => {
            results.push(SanityCheck{
                name: "ShapedDevices.csv Loads?".to_string(),
                success: false,
                comments: format!("{e:?}"),
            });
        }
    }
}

pub fn parent_check(results: &mut Vec<SanityCheck>) {
    if let Ok(net_json) = lqos_config::NetworkJson::load() {
        if net_json.nodes.len() < 2 {
            results.push(SanityCheck{
                name: "Flat Network - Skipping Parent Check".to_string(),
                success: true,
                comments: String::new(),
            });
            return;
        }

        if let Ok(shaped_devices) = lqos_config::ConfigShapedDevices::load() {
            for sd in shaped_devices.devices.iter() {
                if !net_json.nodes.iter().any(|n| n.name == sd.parent_node) {
                    results.push(SanityCheck{
                        name: "Shaped Device Invalid Parent".to_string(),
                        success: false,
                        comments: format!("Device {}/{} is parented to {} - which does not exist", sd.device_name, sd.device_id, sd.parent_node),
                    });
                }
            }
        }
    }
}