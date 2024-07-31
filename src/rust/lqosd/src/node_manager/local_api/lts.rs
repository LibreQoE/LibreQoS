use axum::{Extension, Form, Json};
use serde::Serialize;
use lqos_config::load_config;
use crate::lts2::{ControlSender, FreeTrialDetails};

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
    let (status, trial_expiration) = crate::lts2::get_lts_status();
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
    Extension(lts2): Extension<ControlSender>,
    details: Form<FreeTrialDetails>,
) {
    lts2.send(crate::lts2::LtsCommand::RequestFreeTrial((*details).clone())).unwrap();
}