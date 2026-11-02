use axum::Json;
use axum::http::StatusCode;
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use lqos_config::Config;

fn redacted_api_credentials(mut config: Config) -> Config {
    config.local_api.bearer_token = None;
    config.long_term_stats.license_key = None;
    for key in &mut config.local_api.keys {
        key.token_sha256.clear();
    }
    config
}

fn load_current_config() -> Result<Config, (StatusCode, String)> {
    lqos_config::clear_cached_config();
    lqos_config::load_config()
        .map(|config| config.as_ref().clone())
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to load configuration".to_string(),
            )
        })
}

async fn persist_config_preserving_local_api(config: Config) -> Result<(), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new()
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let mut responses = client
        .request(vec![BusRequest::UpdateLqosdConfigPreserveApiCredentials(
            Box::new(config),
        )])
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    match responses.pop() {
        Some(BusResponse::Ack) => Ok(()),
        Some(BusResponse::Fail(message)) => Err((StatusCode::INTERNAL_SERVER_ERROR, message)),
        Some(response) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unexpected config update response: {response:?}"),
        )),
        None => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "No response received for config update".to_string(),
        )),
    }
}

/// Gets the current configuration as JSON
#[utoipa::path(
    get,
    path = "/config/json",
    summary = "Current LibreQoS configuration as JSON",
    description = "Return the current LibreQoS configuration as JSON. Local API key digests, the legacy local bearer token, and the Insight/API license key are redacted.",
    responses(
        (status = 200, description = "Returns current LibreQoS configuration in JSON."),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Unable to load configuration.", body = String),
    ),
    tag = "Configuration",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_config_as_json() -> Result<Json<Config>, (StatusCode, String)> {
    Ok(Json(redacted_api_credentials(load_current_config()?)))
}

/// Gets the current configuration as TOML
#[utoipa::path(
    get,
    path = "/config/toml",
    summary = "Current LibreQoS configuration as TOML",
    description = "Return the current LibreQoS configuration as TOML. Local API key digests, the legacy local bearer token, and the Insight/API license key are redacted.",
    responses(
        (status = 200, description = "Returns current LibreQoS configuration in TOML.", body = String, content_type = "application/toml"),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Unable to load configuration.", body = String),
    ),
    tag = "Configuration",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_config_as_toml() -> Result<String, (StatusCode, String)> {
    let config = redacted_api_credentials(load_current_config()?);
    toml::to_string_pretty(&config)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

/// Gets a hash of the current config (for backup versioning)
#[utoipa::path(
    get,
    path = "/config/hash",
    summary = "SHA-256 hash of the current configuration for versioning",
    description = "Return a SHA-256 hash of the current configuration for versioning.",
    responses(
        (status = 200, description = "Returns SHA-256 hash of current LibreQoS configuration (hex).", body = String, content_type = "text/plain"),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Unable to load configuration.", body = String),
    ),
    tag = "Configuration",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_config_hash() -> Result<String, (StatusCode, String)> {
    let cfg = tokio::fs::read_to_string("/etc/lqos.conf")
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    use sha2::{Digest, Sha256};
    let result = Sha256::digest(cfg.as_bytes());
    let result = base16ct::lower::encode_string(&result);
    Ok(result)
}

/// Writes a new configuration in TOML format
#[utoipa::path(
    post,
    path = "/config/update_toml",
    summary = "Replace the current configuration with the provided TOML",
    description = "Replace the current configuration with the provided TOML while preserving the authoritative local API keys, legacy local bearer token, and Insight/API license key.",
    responses(
        (status = 200, description = "Configuration saved. Restart lqosd to apply changes."),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Unable to parse or persist configuration.", body = String),
    ),
    tag = "Configuration",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
    request_body = String,
)]
pub async fn write_config_toml(body: String) -> Result<StatusCode, (StatusCode, String)> {
    let proposed: Config = toml::from_str(&body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to parse TOML. {e:?}"),
        )
    })?;
    persist_config_preserving_local_api(proposed).await?;
    Ok(StatusCode::OK)
}

/// Writes a new configuration in JSON format
#[utoipa::path(
    post,
    path = "/config/update_json",
    summary = "Replace the current configuration with the provided JSON",
    description = "Replace the current configuration with the provided JSON while preserving the authoritative local API keys, legacy local bearer token, and Insight/API license key.",
    responses(
        (status = 200, description = "Configuration saved. Restart lqosd to apply changes."),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Unable to parse or persist configuration.", body = String),
    ),
    tag = "Configuration",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
    request_body = String,
)]
pub async fn write_config_json(config: Json<Config>) -> Result<StatusCode, (StatusCode, String)> {
    persist_config_preserving_local_api(config.0).await?;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::redacted_api_credentials;
    use lqos_config::{Config, LocalApiKeyConfig};

    fn config_with_local_credentials() -> Config {
        let mut config = Config::default();
        config.local_api.bearer_token = Some("legacy-secret".to_string());
        config.long_term_stats.license_key = Some("license-secret".to_string());
        config.local_api.keys.push(LocalApiKeyConfig {
            id: "00000000-0000-0000-0000-000000000001".to_string(),
            name: "Monitoring".to_string(),
            token_sha256: "a".repeat(64),
            created_at_unix: 42,
        });
        config
    }

    #[test]
    fn configuration_reads_redact_local_api_credentials_but_keep_metadata() {
        let redacted = redacted_api_credentials(config_with_local_credentials());
        assert_eq!(redacted.local_api.bearer_token, None);
        assert_eq!(redacted.long_term_stats.license_key, None);
        assert_eq!(redacted.local_api.keys.len(), 1);
        assert_eq!(redacted.local_api.keys[0].name, "Monitoring");
        assert!(redacted.local_api.keys[0].token_sha256.is_empty());
        let serialized = toml::to_string(&redacted).expect("redacted config should serialize");
        assert!(!serialized.contains("legacy-secret"));
        assert!(!serialized.contains("license-secret"));
        assert!(!serialized.contains(&"a".repeat(64)));
    }
}
