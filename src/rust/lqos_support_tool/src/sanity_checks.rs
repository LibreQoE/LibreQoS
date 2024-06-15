mod config_sane;
mod interfaces;
mod queues;
mod bridge;
mod net_json;
mod shaped_devices;

use serde::{Deserialize, Serialize};
use crate::console::{error, success};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SanityChecks {
    pub results: Vec<SanityCheck>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SanityCheck {
    pub name: String,
    pub success: bool,
    pub comments: String,
}

pub fn run_sanity_checks() -> anyhow::Result<SanityChecks> {
    println!("Running Sanity Checks");
    let mut results = Vec::new();

    // Run the checks
    config_sane::config_exists(&mut results);
    config_sane::can_load_config(&mut results);
    interfaces::interfaces_exist(&mut results);
    queues::sanity_check_queues(&mut results);
    bridge::check_interface_status(&mut results);
    bridge::check_bridge(&mut results);
    net_json::check_net_json_exists(&mut results);
    net_json::can_we_load_net_json(&mut results);
    net_json::can_we_parse_net_json(&mut results);
    shaped_devices::shaped_devices_exists(&mut results);
    shaped_devices::can_we_read_shaped_devices(&mut results);
    shaped_devices::parent_check(&mut results);

    // Did any fail?
    let mut any_errors = false;
    for s in results.iter() {
        if s.success {
            success(&format!("{} {}", s.name, s.comments));
        } else {
            error(&format!("{}: {}", s.name, s.comments));
            any_errors = true;
        }
    }

    if any_errors {
        error("ERRORS FOUND DURING SANITY CHECK");
    }

    Ok(SanityChecks { results })
}