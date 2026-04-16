//! Local authenticated HTTPS/Caddy setup endpoints for the node manager.

use crate::node_manager::auth::LoginResult;
use axum::{Extension, Json, extract::Host, http::StatusCode};
use lqos_setup::ssl::{SslActionOutcome, SslStatus};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct SetupSslRequest {
    external_hostname: Option<String>,
}

fn ensure_admin(login: LoginResult) -> Result<(), StatusCode> {
    if login == LoginResult::Admin {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

/// Returns the current HTTPS/Caddy state for the running LibreQoS node.
pub(crate) async fn status(
    Extension(login): Extension<LoginResult>,
    Host(host): Host,
) -> Result<Json<SslStatus>, StatusCode> {
    ensure_admin(login)?;
    let config = lqos_config::load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(lqos_setup::ssl::ssl_status(
        config.as_ref(),
        Some(&host),
    )))
}

/// Queues HTTPS enablement for the running LibreQoS node.
pub(crate) async fn setup(
    Extension(login): Extension<LoginResult>,
    Host(host): Host,
    Json(request): Json<SetupSslRequest>,
) -> Result<Json<SslActionOutcome>, (StatusCode, String)> {
    ensure_admin(login).map_err(|status| (status, "Administrator access is required.".into()))?;
    lqos_setup::ssl::enable_runtime_ssl(request.external_hostname, Some(&host))
        .map(Json)
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))
}

/// Queues HTTPS shutdown and restores the direct WebUI listener.
pub(crate) async fn disable(
    Extension(login): Extension<LoginResult>,
    Host(host): Host,
) -> Result<Json<SslActionOutcome>, (StatusCode, String)> {
    ensure_admin(login).map_err(|status| (status, "Administrator access is required.".into()))?;
    lqos_setup::ssl::disable_runtime_ssl(Some(&host))
        .map(Json)
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))
}
