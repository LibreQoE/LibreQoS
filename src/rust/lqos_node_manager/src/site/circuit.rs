use askama::Template;

use axum::{
    Extension,
	http::StatusCode,
    response::IntoResponse,
    extract::{Path,	State},
    routing::get,
    Router,
};
use std::sync::Arc;
use crate::auth::{self, RequireAuth, AuthContext, Credentials, User, Role};
use crate::AppState;
use crate::lqos;
use crate::utils::HtmlTemplate;
use serde::{Serialize};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/:circuit_id", get(circuit_queue).layer(RequireAuth::login()))
}

#[derive(Serialize, Clone)]
pub struct CircuitInfo {
    pub name: String,
    pub capacity: (u64, u64),
}

pub async fn circuit_info(circuit_id: String) -> CircuitInfo {
    let result;
    if let Some(device) = lqos::SHAPED_DEVICES
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
    result
}

#[derive(Template)]
#[template(path = "circuit.html")]
struct CircuitTemplate {
    title: String,
	current_user: User,
	state: AppState,
    circuit_info: CircuitInfo,
}

async fn circuit_queue(
	Extension(user): Extension<User>,
	Path(circuit_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let circuit_info: CircuitInfo = circuit_info(circuit_id).await;
	let template = CircuitTemplate { title: "Circuit Queue".to_string(), current_user: user, state: state, circuit_info: circuit_info };
    HtmlTemplate(template)
}