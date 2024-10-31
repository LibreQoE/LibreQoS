use std::sync::atomic::AtomicU64;
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::error;
use lqos_config::load_config;
use lqos_utils::unix_time::unix_now;

static LAST_VERSION_CHECK: AtomicU64 = AtomicU64::new(0);
const ONE_HOUR_SECONDS: u64 = 60 * 60;
const VERSION_STRING: &str = include_str!("../../../../../VERSION_STRING");

#[derive(Serialize)]
struct VersionCheckRequest {
    current_git_hash: String,
    version_string: String,
    node_id: String,
}

#[derive(Deserialize, Debug)]
pub struct VersionCheckResponse {
    update_available: bool,
}

async fn send_version_check() -> anyhow::Result<VersionCheckResponse> {
    if let Ok(cfg) = load_config() {
        let current_hash = env!("GIT_HASH");
        let request = VersionCheckRequest {
            current_git_hash: current_hash.to_string(),
            version_string: VERSION_STRING.to_string(),
            node_id: cfg.node_id.to_string(),
        };
        let response = reqwest::Client::new()
            .post("https://stats.libreqos.io/api/version_check")
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        Ok(response)
    } else {
        anyhow::bail!("No config");
    }
}

pub async fn version_check() -> Json<String> {
    let last_check = LAST_VERSION_CHECK.load(std::sync::atomic::Ordering::Relaxed);
    if let Ok(now) = unix_now() {
        if now > last_check + ONE_HOUR_SECONDS {
            let res = send_version_check().await;
            if let Ok(response) = send_version_check().await {
                LAST_VERSION_CHECK.store(now, std::sync::atomic::Ordering::Relaxed);

                if response.update_available {
                    return Json(String::from("Update available"));
                }
            } else {
                error!("Unable to send version check");
                error!("{res:?}");
            }
        }
    }
    Json(String::from("All Good"))
}