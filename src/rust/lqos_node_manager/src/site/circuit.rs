use askama::Template;

use axum::{
	http::StatusCode,
    extract::{
		Path,
		State,
    },
    response::{Html, IntoResponse, Extension},
    routing::get,
    Router,
};

use crate::auth;
use crate::AppState;
use crate::lqos::tracker;
use serde::{Deserialize, Serialize};

use lqos_config;

#[derive(Serialize, Clone)]
pub struct CircuitInfo {
  pub name: String,
  pub capacity: (u64, u64),
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/:circuit_id", get(circuit_queue))
}

async fn circuit_queue(
	Extension(user): Extension<auth::User>,
	Path(circuit_id): Path<String>,
    State(state): State<AppState>
) -> impl IntoResponse {
    let mut result;
    if let Some(device) = tracker::SHAPED_DEVICES
        .read()
        .unwrap()
        .devices
        .iter()
        .find(|d| d.circuit_id == circuit_id) {
        result = CircuitInfo {
            name: device.circuit_name.clone(),
            capacity: (
                device.download_max_mbps as u64 * 1_000_000,
                device.upload_max_mbps as u64 * 1_000_000,
            ),
        };
    } else {
        result = CircuitInfo {
            name: "Nameless".to_string(),
            capacity: (1_000_000, 1_000_000),
        };
    }
	let template = CircuitTemplate { title: "Circuit Queue".to_string(), current_user: user, circuit_info: result };
	(StatusCode::OK, Html(template.render().unwrap()).into_response()).into_response()
}

#[derive(Template)]
#[template(path = "circuit.html")]
struct CircuitTemplate {
    title: String,
	current_user: auth::User,
    circuit_info: CircuitInfo,
}