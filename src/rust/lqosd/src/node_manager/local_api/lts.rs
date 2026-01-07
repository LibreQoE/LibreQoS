mod last_24_hours;
mod shaper_status;

use crate::lts2_sys::shared_types::LtsStatus;
use crate::node_manager::auth::LoginResult;
use axum::http::StatusCode;
pub use last_24_hours::*;
use lqos_bus::{BusRequest, bus_request};
use lqos_config::load_config;
use serde::{Deserialize, Serialize};
pub use shaper_status::ShaperStatus;
pub use shaper_status::shaper_status_data;
use std::ops::Deref;
use std::process::Command;
use tracing::{info, warn};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LtsTrialConfig {
    pub node_id: String,
    pub lts_url: Option<String>,
}

pub fn lts_trial_config_data(login: LoginResult) -> Result<LtsTrialConfig, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let cfg = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(LtsTrialConfig {
        node_id: cfg.node_id.clone(),
        lts_url: cfg.long_term_stats.lts_url.clone(),
    })
}

pub(crate) async fn insight_gate() -> Result<(), StatusCode> {
    let (status, _) = crate::lts2_sys::get_lts_license_status_async().await;
    match status {
        LtsStatus::Invalid | LtsStatus::NotChecked => Err(StatusCode::FORBIDDEN),
        _ => Ok(()),
    }
}

pub async fn lts_trial_signup_data(license_key: String) -> Result<(), StatusCode> {
    info!("Received license key, enabling free trial: {}", license_key);
    if license_key == "FAIL" {
        warn!("Free trial request failed");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    info!("Free trial request succeeded, license key: {}", license_key);
    let mut cfg = load_config()
        .expect("Unable to load LibreQoS config")
        .deref()
        .clone();
    cfg.long_term_stats.gather_stats = true;
    cfg.long_term_stats.license_key = Some(license_key);
    bus_request(vec![BusRequest::UpdateLqosdConfig(Box::new(cfg))])
        .await
        .expect("Unable to update lqosd config");
    info!("LQOSD configuration updated with new license key.");
    // Best-effort: ensure the bundled lqos_api also reloads to pick up the new license
    // Ignore errors if systemctl isn't present or permission is denied.
    let _ = Command::new("/bin/systemctl")
        .args(["restart", "lqos_api"])
        .output();
    Ok(())
}
