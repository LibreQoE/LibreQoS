use axum::Json;
use axum::http::StatusCode;
use serde_json::Value;
use std::path::PathBuf;

fn network_json_path() -> Result<PathBuf, (StatusCode, String)> {
    lqos_config::NetworkJson::path().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to determine network.json path: {e}"),
        )
    })
}

/// Reads the `network.json` file as raw text
#[utoipa::path(
    get,
    path = "/network_json/text",
    summary = "Network.json as raw text",
    description = "Return network.json as raw text.",
    responses(
        (status = 200, description = "Returns the current network.json as UTF-8 text.", body = String, content_type = "text/plain"),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Unable to read network.json.", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_network_json_as_text() -> Result<String, (StatusCode, String)> {
    let path = network_json_path()?;
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to read network.json. {e:?}"),
        )
    })?;
    Ok(content)
}

/// Reads the `network.json` file and deserializes it to JSON
#[utoipa::path(
    get,
    path = "/network_json/json",
    summary = "Network.json parsed as JSON",
    description = "Return network.json parsed as JSON.",
    responses(
        (status = 200, description = "Returns the current network.json parsed into JSON.", body = serde_json::Value),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Unable to read or parse network.json.", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_network_json_as_json() -> Result<Json<Value>, (StatusCode, String)> {
    let raw = get_network_json_as_text().await?;
    let json: Value = serde_json::from_str(&raw).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to parse network.json. {e:?}"),
        )
    })?;
    Ok(Json(json))
}

/// Ownership gate for the retired topology-write routes.
///
/// These routes never write: they only tell the caller who owns the current
/// topology. A native integration (or a non-legacy `topology_import.json`)
/// owns it; otherwise the Topology Manager editor does.
fn legacy_topology_write_response() -> (StatusCode, String) {
    let editor = || {
        (
            StatusCode::CONFLICT,
            "topology_edit_requires_editor: use the LibreQoS Topology Manager.".to_string(),
        )
    };
    let Ok(config) = lqos_config::load_config() else {
        return editor();
    };

    let canonical_path =
        config.topology_state_file_path(lqos_config::TOPOLOGY_CANONICAL_STATE_FILENAME);
    if canonical_path.exists() {
        match lqos_config::TopologyCanonicalStateFile::load(config.as_ref()) {
            Ok(state)
                if state.ingress_kind
                    == lqos_config::TopologyCanonicalIngressKind::NativeIntegration =>
            {
                return topology_managed_response(&state.source);
            }
            Ok(_) => {}
            // Unreadable state: refuse rather than guess ownership.
            Err(_) => return editor(),
        }
    }

    let import_path = config.topology_state_file_path(lqos_config::TOPOLOGY_IMPORT_FILENAME);
    if import_path.exists()
        && let Ok(raw) = std::fs::read_to_string(&import_path)
        && let Ok(imported) = serde_json::from_str::<Value>(&raw)
    {
        let source = imported
            .get("source")
            .and_then(Value::as_str)
            .or_else(|| {
                imported
                    .get("imported")
                    .and_then(|value| value.get("source"))
                    .and_then(Value::as_str)
            })
            .unwrap_or_default();
        if !source.is_empty() && !source.starts_with("legacy/") {
            return topology_managed_response(source);
        }
    }

    editor()
}

fn topology_managed_response(source: &str) -> (StatusCode, String) {
    (
        StatusCode::CONFLICT,
        format!(
            "topology_managed_by_integration: current topology source is {source}; use the Topology Manager."
        ),
    )
}

#[utoipa::path(
    post,
    path = "/network_json/update",
    summary = "Retired: replace network.json (409 stub)",
    description = "Retired topology write. This route never writes; it returns 409 and names the current topology owner.",
    responses(
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 409, description = "Topology writes are owned by the Topology Manager (editor or integration).", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn legacy_network_json_update_stub() -> (StatusCode, String) {
    legacy_topology_write_response()
}

#[utoipa::path(
    post,
    path = "/network_json/node",
    summary = "Retired: add a network.json node (409 stub)",
    description = "Retired topology write. This route never writes; it returns 409 and names the current topology owner.",
    responses(
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 409, description = "Topology writes are owned by the Topology Manager (editor or integration).", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn legacy_network_json_add_node_stub() -> (StatusCode, String) {
    legacy_topology_write_response()
}

#[utoipa::path(
    delete,
    path = "/network_json/node/{node_name}",
    summary = "Retired: remove a network.json node (409 stub)",
    description = "Retired topology write. This route never writes; it returns 409 and names the current topology owner.",
    responses(
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 409, description = "Topology writes are owned by the Topology Manager (editor or integration).", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("node_name" = String, Path, description = "Node name (ignored by the retired stub).", example = "Town"),
    ),
)]
pub async fn legacy_network_json_remove_node_stub() -> (StatusCode, String) {
    legacy_topology_write_response()
}

#[utoipa::path(
    post,
    path = "/network/reparent_site",
    summary = "Retired: reparent a site (409 stub)",
    description = "Retired topology write. This route never writes; it returns 409 and names the current topology owner.",
    responses(
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 409, description = "Topology writes are owned by the Topology Manager (editor or integration).", body = String),
    ),
    tag = "Topology & Files",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn legacy_network_reparent_site_stub() -> (StatusCode, String) {
    legacy_topology_write_response()
}

/// Resolve a parent name against the published flat operator view.
///
/// `NetworkJson::load()` is root-only in v3, so callers must use the
/// `TopologyOperatorViewFile` artifact instead. A display name claimed by more
/// than one node is a loud conflict, never a silent pick.
pub(crate) async fn published_network_has_node(
    site_name: &str,
) -> Result<bool, (StatusCode, String)> {
    let site_name = site_name.to_string();
    let view = tokio::task::spawn_blocking(move || {
        let config =
            lqos_config::load_config().map_err(|e| format!("Unable to load config: {e}"))?;
        lqos_config::TopologyOperatorViewFile::load(config.as_ref())
            .map_err(|e| format!("Unable to load topology operator view: {e}"))
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Topology operator view probe failed: {e}"),
        )
    })?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    if view.ambiguous_base_names.contains(&site_name) {
        return Err((
            StatusCode::CONFLICT,
            format!("Node name is ambiguous in the topology operator view: {site_name}"),
        ));
    }
    Ok(view.nodes.iter().any(|node| node.name == site_name))
}
