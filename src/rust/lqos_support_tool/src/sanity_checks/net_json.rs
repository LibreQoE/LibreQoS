use std::path::Path;
use serde_json::Value;
use lqos_config::load_config;
use crate::sanity_checks::SanityCheck;

pub fn check_net_json_exists(results: &mut Vec<SanityCheck>) {
    if let Ok(cfg) = load_config() {
        let path = Path::new(&cfg.lqos_directory).join("network.json");
        if path.exists() {
            results.push(SanityCheck{
                name: "network.json exists".to_string(),
                success: true,
                comments: "".to_string(),
            });
        } else {
            results.push(SanityCheck{
                name: "network.json exists".to_string(),
                success: false,
                comments: format!("File not found at {:?}", path),
            });
        }
    }
}

pub fn can_we_load_net_json(results: &mut Vec<SanityCheck>) {
    if let Ok(cfg) = load_config() {
        let path = Path::new(&cfg.lqos_directory).join("network.json");
        if path.exists() {
            if let Ok(str) = std::fs::read_to_string(path) {
                match serde_json::from_str::<Value>(&str) {
                    Ok(json) => {
                        results.push(SanityCheck{
                            name: "network.json is parseable JSON".to_string(),
                            success: true,
                            comments: "".to_string(),
                        });
                    }
                    Err(e) => {
                        results.push(SanityCheck{
                            name: "network.json is parseable JSON".to_string(),
                            success: false,
                            comments: format!("{e:?}"),
                        });
                    }
                }
            }
        }
    }
}

pub fn can_we_parse_net_json(results: &mut Vec<SanityCheck>) {
    match lqos_config::NetworkJson::load() {
        Ok(json) => {
            results.push(SanityCheck{
                name: "network.json is valid JSON".to_string(),
                success: true,
                comments: "".to_string(),
            });
        }
        Err(e) => {
            results.push(SanityCheck{
                name: "network.json is valid JSON".to_string(),
                success: false,
                comments: format!("{e:?}"),
            });
        }
    }
}