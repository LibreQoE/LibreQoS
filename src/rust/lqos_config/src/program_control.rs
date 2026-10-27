use serde::Serialize;
use std::{path::Path, path::PathBuf};
use thiserror::Error;
use tracing::error;
use uuid::Uuid;

/// The state-directory category the scheduler owns for its own runtime files.
const RELOAD_REQUEST_CATEGORY: &str = "scheduler";
/// The reload-request token filename consumed by the scheduler's 1 s tick.
const RELOAD_REQUEST_FILENAME: &str = "reload_request.json";
/// The immediate ack returned to the operator when a reload is requested.
pub const RELOAD_REQUEST_ACK: &str =
    "Reload requested — re-syncing the integration and applying";

/// The on-demand source-refresh request token written for the scheduler.
#[derive(Serialize)]
struct ReloadRequestToken<'a> {
    /// Unix seconds the request was written.
    requested_at: u64,
    /// Operator/role that requested the reload.
    requested_by: &'a str,
    /// Idempotency key: the scheduler reloads once per unseen nonce.
    nonce: String,
}

/// Returns the scheduler reload-request token path for `config`.
pub fn reload_request_path(config: &crate::Config) -> PathBuf {
    config
        .resolved_state_directory()
        .join(RELOAD_REQUEST_CATEGORY)
        .join(RELOAD_REQUEST_FILENAME)
}

/// Writes a fresh reload-request token to `path` (atomic tmp + rename).
///
/// This is the testable, config-free core of [`request_libreqos_reload`].
pub fn write_reload_request(path: &Path, requested_by: &str) -> Result<(), ProgramControlError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            error!(
                "Unable to create the reload-request directory {}: {err}",
                parent.display()
            );
            ProgramControlError::RequestWriteFailed
        })?;
    }
    let token = ReloadRequestToken {
        requested_at: now_unix_seconds(),
        requested_by,
        nonce: Uuid::new_v4().to_string(),
    };
    let serialized = serde_json::to_vec_pretty(&token)
        .map_err(|_| ProgramControlError::RequestWriteFailed)?;
    let temp_path = path.with_extension("json.tmp");
    std::fs::write(&temp_path, &serialized).map_err(|err| {
        error!(
            "Unable to write the reload request {}: {err}",
            temp_path.display()
        );
        ProgramControlError::RequestWriteFailed
    })?;
    std::fs::rename(&temp_path, path).map_err(|err| {
        error!(
            "Unable to publish the reload request {}: {err}",
            path.display()
        );
        ProgramControlError::RequestWriteFailed
    })?;
    Ok(())
}

/// Requests an on-demand source refresh from the scheduler (the single apply
/// owner): writes the `{requested_at, requested_by, nonce}` token and returns an
/// immediate ack. The DPDK bridge is never restarted by this control.
pub fn request_libreqos_reload(requested_by: &str) -> Result<String, ProgramControlError> {
    let config = crate::load_config().map_err(|_| ProgramControlError::ConfigLoadError)?;
    write_reload_request(&reload_request_path(&config), requested_by)?;
    Ok(RELOAD_REQUEST_ACK.to_string())
}

fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Error, Debug)]
pub enum ProgramControlError {
    #[error("Unable to load lqos configuration from /etc")]
    ConfigLoadError,
    #[error("Unable to write the scheduler reload request")]
    RequestWriteFailed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique}"));
        std::fs::create_dir_all(&path).expect("temp directory should be creatable");
        path
    }

    #[test]
    fn write_reload_request_emits_the_token_schema() {
        let dir = unique_temp_dir("lqos-reload-request");
        let path = dir.join("scheduler").join("reload_request.json");

        write_reload_request(&path, "admin").expect("request should write");

        let raw = std::fs::read_to_string(&path).expect("token should exist");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("token should be JSON");
        assert!(parsed["requested_at"].as_u64().is_some_and(|t| t > 0));
        assert_eq!(parsed["requested_by"].as_str(), Some("admin"));
        let nonce = parsed["nonce"].as_str().expect("nonce present");
        assert!(!nonce.is_empty(), "nonce is the idempotency key");
        assert!(
            !dir.join("scheduler").join("reload_request.json.tmp").exists(),
            "the atomic temp file must be renamed away"
        );
    }

    #[test]
    fn write_reload_request_rotates_the_nonce_on_each_request() {
        let dir = unique_temp_dir("lqos-reload-request-rotate");
        let path = dir.join("scheduler").join("reload_request.json");

        write_reload_request(&path, "admin").expect("first request");
        let first: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        write_reload_request(&path, "admin").expect("second request");
        let second: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert_ne!(first["nonce"], second["nonce"], "each click is a new request");
    }
}
