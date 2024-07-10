use axum::Json;
use serde::Serialize;
use lqos_config::load_config;

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
}

pub async fn stats_check() -> Json<StatsCheckAction> {
    let mut response = StatsCheckAction {
        action: StatsCheckResponse::DoNothing,
        node_id: String::new(),
    };

    if let Ok(cfg) = load_config() {
        if !cfg.long_term_stats.gather_stats {
            response = StatsCheckAction {
                action: StatsCheckResponse::NotSetup,
                node_id: cfg.node_id.to_string(),
            };
        } else {
            // Stats are enabled
            response = StatsCheckAction {
                action: StatsCheckResponse::GoodToGo,
                node_id: cfg.node_id.to_string(),
            };
        }
    }

    Json(response)
}