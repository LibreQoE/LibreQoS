mod shaper_status;
mod last_24_hours;
mod rest_client;

use std::ops::Deref;
use axum::{Form, Json};
use axum::response::Redirect;
use tracing::{info, warn};
use serde::Serialize;
use lqos_bus::{bus_request, BusRequest};
use lqos_config::load_config;
pub use shaper_status::shaper_status_from_lts;
pub use last_24_hours::*;
use crate::lts2_sys::shared_types::FreeTrialDetails;

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

pub async fn lts_trial_signup(
    details: Form<FreeTrialDetails>,
) -> Redirect {
    let license_key = crate::lts2_sys::request_free_trial((*details).clone()).unwrap();

    info!("Received license key, enabling free trial: {}", license_key);
    if license_key == "FAIL" {
        warn!("Free trial request failed");
        Redirect::temporary("../lts_trail_fail.html")
    } else {
        let mut cfg = load_config().unwrap().deref().clone();
        cfg.long_term_stats.license_key = Some(license_key);
        bus_request(vec![BusRequest::UpdateLqosdConfig(Box::new(cfg))])
            .await
            .unwrap();
        Redirect::temporary("../lts_trial_success.html")
    }
}