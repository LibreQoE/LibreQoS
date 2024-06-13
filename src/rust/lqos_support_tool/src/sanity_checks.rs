mod config_sane;
mod interfaces;
mod queues;

use serde::{Deserialize, Serialize};
use crate::console::{error, success};

#[derive(Debug, Serialize, Deserialize)]
pub struct SanityChecks {
    results: Vec<SanityCheck>,
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

    // Did any fail?
    let mut any_errors = false;
    for s in results.iter() {
        if s.success {
            success(&s.name);
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