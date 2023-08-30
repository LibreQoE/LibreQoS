use lqos_config::EtcLqos;
use lqos_utils::unix_time::unix_now;
use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};

static LAST_VERSION_CHECK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const ONE_HOUR_SECONDS: u64 = 60 * 60;
const VERSION_STRING: &str = include_str!("../../../VERSION_STRING");

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
struct VersionCheckRequest {
    current_git_hash: String,
    version_string: String,
    node_id: String,
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct VersionCheckResponse {
    update_available: bool,
}

async fn send_version_check() -> anyhow::Result<VersionCheckResponse> {
    if let Ok(cfg) = EtcLqos::load() {
        let current_hash = env!("GIT_HASH");
        let request = VersionCheckRequest {
            current_git_hash: current_hash.to_string(),
            version_string: VERSION_STRING.to_string(),
            node_id: cfg.node_id.unwrap_or("(not configured)".to_string()),
        };
        let response = reqwest::Client::new()
            .post("https://stats.libreqos.io/api/version_check")
            .json(&request)
            .send()
            .await?
            .json::<VersionCheckResponse>()
            .await?;

        Ok(response)
    } else {
        anyhow::bail!("No config");
    }
}

#[get("/api/version_check")]
pub async fn version_check() -> Json<String> {
    let last_check = LAST_VERSION_CHECK.load(std::sync::atomic::Ordering::Relaxed);
    if let Ok(now) = unix_now() {
        if now > last_check + ONE_HOUR_SECONDS {
            if let Ok(response) = send_version_check().await {
                LAST_VERSION_CHECK.store(now, std::sync::atomic::Ordering::Relaxed);

                if response.update_available {
                    return Json(String::from("Update available"));
                }
            }
        }
    }
    Json(String::from("All Good"))
}

#[get("/api/stats_check")]
pub async fn stats_check() -> Json<String> {
    Json(String::from("No"))
}
