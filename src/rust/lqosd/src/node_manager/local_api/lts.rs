mod last_24_hours;
mod rest_client;
mod shaper_status;

use crate::lts2_sys::shared_types::FreeTrialDetails;
use axum::response::Redirect;
use axum::{Form, Json};
pub use last_24_hours::*;
use lqos_bus::{BusRequest, bus_request};
use lqos_config::load_config;
use serde::{Deserialize, Serialize};
pub use shaper_status::shaper_status_from_lts;
use std::ops::Deref;
use axum::http::StatusCode;
use tracing::{info, warn};
use std::process::Command;

#[derive(Serialize)]
pub enum StatsCheckResponse {
    DoNothing,
    NotSetup,
    GoodToGo,
}

#[derive(Serialize)]
pub struct StatsCheckAction {
    action: StatsCheckResponse,
    node_id: String,
    node_name: String,
}

pub async fn stats_check() -> Json<StatsCheckAction> {
    let (status, trial_expiration) = crate::lts2_sys::get_lts_license_status_async().await;
    println!("{:?}, {trial_expiration:?}", status);
    let mut response = StatsCheckAction {
        action: StatsCheckResponse::DoNothing,
        node_id: String::new(),
        node_name: "LQOS Node".to_string(),
    };

    if let Ok(cfg) = load_config() {
        if !cfg.long_term_stats.gather_stats {
            response = StatsCheckAction {
                action: StatsCheckResponse::NotSetup,
                node_id: cfg.node_id.to_string(),
                node_name: cfg.node_name.to_string(),
            };
        } else {
            // Stats are enabled
            response = StatsCheckAction {
                action: StatsCheckResponse::GoodToGo,
                node_id: cfg.node_id.to_string(),
                node_name: cfg.node_name.to_string(),
            };
        }
    }

    Json(response)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct LicenseKey {
    pub license_key: String,
}

pub async fn lts_trial_signup(details: Json<LicenseKey>) -> StatusCode {
    info!("Received free trial signup request: {:?}", details);
    let license_key = details.license_key.clone();

    info!("Received license key, enabling free trial: {}", license_key);
    if license_key == "FAIL" {
        warn!("Free trial request failed");
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        info!("Free trial request succeeded, license key: {}", license_key);
        let mut cfg = load_config().unwrap().deref().clone();
        cfg.long_term_stats.gather_stats = true;
        cfg.long_term_stats.license_key = Some(license_key);
        bus_request(vec![BusRequest::UpdateLqosdConfig(Box::new(cfg))])
            .await
            .unwrap();
        info!("LQOSD configuration updated with new license key.");
        // Best-effort: ensure the bundled lqos_api also reloads to pick up the new license
        // Ignore errors if systemctl isn't present or permission is denied.
        let _ = Command::new("/bin/systemctl")
            .args(["restart", "lqos_api"])
            .output();
        std::process::exit(0);
        //StatusCode::OK
    }
}
