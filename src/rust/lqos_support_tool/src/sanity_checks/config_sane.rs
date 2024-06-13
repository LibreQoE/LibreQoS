use std::path::Path;
use lqos_config::load_config;
use crate::sanity_checks::SanityCheck;

pub fn config_exists(results: &mut Vec<SanityCheck>) {
    let path = Path::new("/etc/lqos.conf");
    let mut result = SanityCheck {
        name: "Config File Exists".to_string(),
        ..Default::default()
    };
    if path.exists() {
        result.success = true;
    } else {
        result.success = false;
        result.comments = "/etc/lqos.conf could not be opened".to_string();
    }

    results.push(result);
}

pub fn can_load_config(results: &mut Vec<SanityCheck>) {
    let mut result = SanityCheck {
        name: "Config File Can Be Loaded".to_string(),
        ..Default::default()
    };
    let cfg = load_config();
    if cfg.is_ok() {
        result.success = true;
    } else {
        result.success = false;
        result.comments = "Configuration file could not be loaded".to_string();
    }
    results.push(result);
}