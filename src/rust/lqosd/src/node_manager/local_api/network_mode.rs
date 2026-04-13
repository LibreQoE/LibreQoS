use crate::node_manager::auth::{LoginResult, get_username};
use axum::Json;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum_extra::extract::CookieJar;
use lqos_config::Config;
use lqos_netplan_helper::protocol::{ApplyMode, ApplyRequest, ApplyResponse, HelperStatus};
use lqos_netplan_helper::transaction::{
    HelperPaths, PendingChildren, apply_transaction, confirm_transaction, helper_status,
    inspect_with_paths, retry_shaping_transaction, revert_transaction, rollback_transaction,
};
pub use lqos_netplan_helper::{NetworkModeInspection, inspect_network_mode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, warn};

#[derive(Clone, Debug)]
pub struct NetworkModeApiState {
    paths: Arc<HelperPaths>,
    pending_children: Arc<Mutex<PendingChildren>>,
}

impl Default for NetworkModeApiState {
    fn default() -> Self {
        let paths = Arc::new(HelperPaths::default());
        let mut pending_children = PendingChildren::default();
        if let Err(err) = helper_status(paths.as_ref(), &mut pending_children) {
            warn!("Unable to initialize network-mode helper state: {err}");
        }
        Self {
            paths,
            pending_children: Arc::new(Mutex::new(pending_children)),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct NetworkModeInspectRequest {
    pub config: Config,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NetworkModeApplyRequest {
    pub config: Config,
    #[serde(default)]
    pub mode: ApplyMode,
    #[serde(default)]
    pub confirm_dangerous_changes: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NetworkModeConfirmRequest {
    pub operation_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NetworkModeRollbackRequest {
    pub backup_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct NetworkModeStateResponse {
    pub helper_status: HelperStatus,
}

fn unauthorized() -> (StatusCode, Json<ApplyResponse>) {
    (
        StatusCode::FORBIDDEN,
        Json(ApplyResponse {
            ok: false,
            message: "Unauthorized".to_string(),
            operation: None,
            last_backup_id: None,
        }),
    )
}

fn helper_validation_error_response(message: String) -> (StatusCode, Json<ApplyResponse>) {
    warn!("Bridge helper request failed: {message}");
    (
        StatusCode::BAD_REQUEST,
        Json(ApplyResponse {
            ok: false,
            message,
            operation: None,
            last_backup_id: None,
        }),
    )
}

fn helper_internal_error_response(message: String) -> (StatusCode, Json<ApplyResponse>) {
    error!("Bridge helper internal error: {message}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApplyResponse {
            ok: false,
            message,
            operation: None,
            last_backup_id: None,
        }),
    )
}

fn merge_network_mode(candidate: Config) -> Result<Config, StatusCode> {
    let live = lqos_config::load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut merged = (*live).clone();
    merged.bridge = candidate.bridge;
    merged.single_interface = candidate.single_interface;
    Ok(merged)
}

async fn run_network_mode_inspection(
    state: &NetworkModeApiState,
    config: Config,
) -> Result<NetworkModeInspection, StatusCode> {
    let paths = state.paths.clone();
    tokio::task::spawn_blocking(move || inspect_with_paths(paths.as_ref(), &config))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn run_pending_operation<T, F>(
    state: &NetworkModeApiState,
    operation: F,
) -> Result<T, StatusCode>
where
    T: Send + 'static,
    F: FnOnce(&HelperPaths, &mut PendingChildren) -> anyhow::Result<T> + Send + 'static,
{
    let paths = state.paths.clone();
    let pending_children = state.pending_children.clone();
    tokio::task::spawn_blocking(move || {
        let mut pending = pending_children.blocking_lock();
        operation(paths.as_ref(), &mut pending)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn status(
    State(state): State<NetworkModeApiState>,
    Extension(login): Extension<LoginResult>,
) -> Result<Json<NetworkModeStateResponse>, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let helper_status = run_pending_operation(&state, helper_status).await?;

    Ok(Json(NetworkModeStateResponse { helper_status }))
}

pub async fn inspect(
    State(state): State<NetworkModeApiState>,
    Extension(login): Extension<LoginResult>,
    Json(body): Json<NetworkModeInspectRequest>,
) -> Result<Json<NetworkModeInspection>, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let merged = merge_network_mode(body.config)?;
    Ok(Json(run_network_mode_inspection(&state, merged).await?))
}

pub async fn apply(
    State(state): State<NetworkModeApiState>,
    jar: CookieJar,
    Extension(login): Extension<LoginResult>,
    Json(body): Json<NetworkModeApplyRequest>,
) -> (StatusCode, Json<ApplyResponse>) {
    if login != LoginResult::Admin {
        return unauthorized();
    }

    let username = get_username(&jar).await;
    let merged = match merge_network_mode(body.config) {
        Ok(config) => config,
        Err(status) => {
            return (
                status,
                Json(ApplyResponse {
                    ok: false,
                    message: "Unable to load the live LibreQoS configuration".to_string(),
                    operation: None,
                    last_backup_id: None,
                }),
            );
        }
    };
    let result = tokio::task::spawn_blocking({
        let paths = state.paths.clone();
        let pending_children = state.pending_children.clone();
        move || {
            let mut pending = pending_children.blocking_lock();
            apply_transaction(
                paths.as_ref(),
                &mut pending,
                ApplyRequest {
                    config: merged,
                    source: "ui".to_string(),
                    operator_username: Some(username),
                    mode: body.mode,
                    confirm_dangerous_changes: body.confirm_dangerous_changes,
                },
            )
        }
    })
    .await;
    match result {
        Ok(Ok(response)) => (StatusCode::OK, Json(response)),
        Ok(Err(err)) => helper_validation_error_response(err.to_string()),
        Err(err) => helper_internal_error_response(format!("Bridge helper task failed: {err}")),
    }
}

pub async fn confirm(
    State(state): State<NetworkModeApiState>,
    Extension(login): Extension<LoginResult>,
    Json(body): Json<NetworkModeConfirmRequest>,
) -> (StatusCode, Json<ApplyResponse>) {
    if login != LoginResult::Admin {
        return unauthorized();
    }

    let result = tokio::task::spawn_blocking({
        let paths = state.paths.clone();
        let pending_children = state.pending_children.clone();
        let operation_id = body.operation_id.clone();
        move || {
            let mut pending = pending_children.blocking_lock();
            confirm_transaction(paths.as_ref(), &mut pending, &operation_id)
        }
    })
    .await;
    match result {
        Ok(Ok(response)) => (StatusCode::OK, Json(response)),
        Ok(Err(err)) => helper_validation_error_response(err.to_string()),
        Err(err) => helper_internal_error_response(format!("Bridge helper task failed: {err}")),
    }
}

pub async fn revert(
    State(state): State<NetworkModeApiState>,
    Extension(login): Extension<LoginResult>,
    Json(body): Json<NetworkModeConfirmRequest>,
) -> (StatusCode, Json<ApplyResponse>) {
    if login != LoginResult::Admin {
        return unauthorized();
    }

    let result = tokio::task::spawn_blocking({
        let paths = state.paths.clone();
        let pending_children = state.pending_children.clone();
        let operation_id = body.operation_id.clone();
        move || {
            let mut pending = pending_children.blocking_lock();
            revert_transaction(paths.as_ref(), &mut pending, &operation_id)
        }
    })
    .await;
    match result {
        Ok(Ok(response)) => (StatusCode::OK, Json(response)),
        Ok(Err(err)) => helper_validation_error_response(err.to_string()),
        Err(err) => helper_internal_error_response(format!("Bridge helper task failed: {err}")),
    }
}

pub async fn rollback(
    State(state): State<NetworkModeApiState>,
    Extension(login): Extension<LoginResult>,
    Json(body): Json<NetworkModeRollbackRequest>,
) -> (StatusCode, Json<ApplyResponse>) {
    if login != LoginResult::Admin {
        return unauthorized();
    }

    let result = tokio::task::spawn_blocking({
        let paths = state.paths.clone();
        let pending_children = state.pending_children.clone();
        let backup_id = body.backup_id.clone();
        move || {
            let mut pending = pending_children.blocking_lock();
            rollback_transaction(paths.as_ref(), &mut pending, &backup_id)
        }
    })
    .await;
    match result {
        Ok(Ok(response)) => (StatusCode::OK, Json(response)),
        Ok(Err(err)) => helper_validation_error_response(err.to_string()),
        Err(err) => helper_internal_error_response(format!("Bridge helper task failed: {err}")),
    }
}

pub async fn retry_shaping(
    State(state): State<NetworkModeApiState>,
    Extension(login): Extension<LoginResult>,
) -> (StatusCode, Json<ApplyResponse>) {
    if login != LoginResult::Admin {
        return unauthorized();
    }

    let result = tokio::task::spawn_blocking({
        let paths = state.paths.clone();
        let pending_children = state.pending_children.clone();
        move || {
            let mut pending = pending_children.blocking_lock();
            retry_shaping_transaction(paths.as_ref(), &mut pending)
        }
    })
    .await;
    match result {
        Ok(Ok(response)) => (StatusCode::OK, Json(response)),
        Ok(Err(err)) => helper_internal_error_response(err.to_string()),
        Err(err) => helper_internal_error_response(format!("Bridge helper task failed: {err}")),
    }
}
