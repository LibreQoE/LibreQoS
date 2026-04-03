mod last_24_hours;
mod shaper_status;

use crate::lts2_sys::lts2_client::{LicenseStatus, set_license_status};
use crate::lts2_sys::shared_types::LtsStatus;
use crate::node_manager::auth::LoginResult;
use crate::node_manager::local_api::circuit_count;
use axum::http::StatusCode;
pub use last_24_hours::*;
use lqos_bus::{BusRequest, bus_request};
use lqos_config::load_config;
use serde::{Deserialize, Serialize};
pub use shaper_status::ShaperStatus;
pub use shaper_status::shaper_status_data;
use std::process::Command;
use std::time::Duration;
use tracing::{debug, info, warn};
use uuid::Uuid;

const SIGNUP_POLL_INTERVAL: Duration = Duration::from_secs(10);
const INSIGHT_FREE_TRIAL_STATUS_CODE: i32 = 2;
const INSIGHT_TRIAL_DAYS_HINT: i32 = 30;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LtsTrialConfig {
    pub node_id: String,
    pub lts_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct SignupStartRequest {
    node_id: String,
    node_name: String,
    active_circuits: usize,
}

#[derive(Debug, Deserialize)]
struct SignupStartResponse {
    claim_id: Uuid,
}

#[derive(Debug, Serialize)]
struct SignupCheckRequest {
    claim_id: String,
}

#[derive(Debug, Deserialize)]
struct SignupCheckResponse {
    status: String,
    #[serde(default)]
    account_key: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignupCheckState {
    Pending,
    Provisioned { account_key: Uuid },
    Expired,
}

pub fn lts_trial_config_data(login: LoginResult) -> Result<LtsTrialConfig, StatusCode> {
    if login != LoginResult::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let cfg = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(LtsTrialConfig {
        node_id: cfg.node_id.clone(),
        lts_url: cfg.long_term_stats.lts_url.clone(),
    })
}

fn normalize_insight_base_url(lts_url: Option<&str>) -> String {
    let mut base_url = lts_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("https://insight.libreqos.com/")
        .to_string();

    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        base_url = format!("https://{base_url}");
    }

    while base_url.ends_with('/') {
        base_url.pop();
    }

    if let Some(stripped) = base_url.strip_suffix("/signup-api") {
        base_url = stripped.to_string();
    }

    format!("{base_url}/")
}

fn build_insight_url(lts_url: Option<&str>, path: &str) -> String {
    let path = path.trim_start_matches('/');
    format!("{}{}", normalize_insight_base_url(lts_url), path)
}

fn insight_http_client() -> Result<reqwest::Client, StatusCode> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn classify_signup_check_response(response: SignupCheckResponse) -> Result<SignupCheckState, ()> {
    match response.status.as_str() {
        "pending" => Ok(SignupCheckState::Pending),
        "expired" => Ok(SignupCheckState::Expired),
        "provisioned" => response
            .account_key
            .map(|account_key| SignupCheckState::Provisioned { account_key })
            .ok_or(()),
        _ => Err(()),
    }
}

async fn apply_insight_license(
    license_key: String,
    restart_lqos_api: bool,
) -> Result<(), StatusCode> {
    let license_key = license_key.trim().to_string();
    if license_key.is_empty() {
        return Err(StatusCode::BAD_GATEWAY);
    }

    let mut cfg = load_config()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .as_ref()
        .clone();
    cfg.long_term_stats.gather_stats = true;
    cfg.long_term_stats.license_key = Some(license_key.clone());
    bus_request(vec![BusRequest::UpdateLqosdConfig(Box::new(cfg))])
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    set_license_status(LicenseStatus {
        license_type: INSIGHT_FREE_TRIAL_STATUS_CODE,
        trial_expires: INSIGHT_TRIAL_DAYS_HINT,
    });

    info!("LQOSD configuration updated with new Insight license key.");

    if restart_lqos_api {
        let _ = Command::new("/bin/systemctl")
            .args(["restart", "lqos_api"])
            .output();
    }

    Ok(())
}

pub async fn lts_trial_start_signup_data() -> Result<String, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let start_url = build_insight_url(config.long_term_stats.lts_url.as_deref(), "/su/start");
    let request = SignupStartRequest {
        node_id: config.node_id.clone(),
        node_name: config.node_name.clone(),
        active_circuits: circuit_count::circuit_count_data().count,
    };
    debug!(
        node_id = %request.node_id,
        active_circuits = request.active_circuits,
        url = %start_url,
        "starting Insight signup session"
    );

    let client = insight_http_client()?;

    let response = client
        .post(&start_url)
        .json(&request)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    if !response.status().is_success() {
        warn!(
            status = %response.status(),
            url = %start_url,
            "Insight signup start request failed"
        );
        return Err(StatusCode::BAD_GATEWAY);
    }

    let response = response
        .json::<SignupStartResponse>()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    Ok(response.claim_id.to_string())
}

pub fn spawn_signup_poll_loop(claim_id: String) {
    tokio::spawn(async move {
        if let Err(status) = poll_signup_until_complete(claim_id.clone()).await {
            warn!(
                claim_id = %claim_id,
                status = %status,
                "Insight signup poll loop stopped with an error"
            );
        }
    });
}

async fn poll_signup_until_complete(claim_id: String) -> Result<(), StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let check_url = build_insight_url(config.long_term_stats.lts_url.as_deref(), "/su/check");
    let client = insight_http_client()?;
    let request = SignupCheckRequest {
        claim_id: claim_id.clone(),
    };

    loop {
        tokio::time::sleep(SIGNUP_POLL_INTERVAL).await;

        let response = match client.post(&check_url).json(&request).send().await {
            Ok(response) => response,
            Err(error) => {
                warn!(
                    claim_id = %claim_id,
                    error = ?error,
                    "Insight signup status poll failed; retrying"
                );
                continue;
            }
        };

        if !response.status().is_success() {
            warn!(
                claim_id = %claim_id,
                status = %response.status(),
                "Insight signup status poll returned non-success; retrying"
            );
            continue;
        }

        let response = match response.json::<SignupCheckResponse>().await {
            Ok(response) => response,
            Err(error) => {
                warn!(
                    claim_id = %claim_id,
                    error = ?error,
                    "Insight signup status poll returned invalid JSON; retrying"
                );
                continue;
            }
        };

        match classify_signup_check_response(response) {
            Ok(SignupCheckState::Pending) => {}
            Ok(SignupCheckState::Expired) => {
                debug!(claim_id = %claim_id, "Insight signup claim expired");
                return Ok(());
            }
            Ok(SignupCheckState::Provisioned { account_key }) => {
                match apply_insight_license(account_key.to_string(), false).await {
                    Ok(()) => {
                        info!(
                            claim_id = %claim_id,
                            account_key = %account_key,
                            "Applied Insight license from signup flow"
                        );
                        return Ok(());
                    }
                    Err(status) => {
                        warn!(
                            claim_id = %claim_id,
                            account_key = %account_key,
                            status = %status,
                            "Failed to apply Insight license; stopping signup poll loop"
                        );
                        return Err(status);
                    }
                }
            }
            Err(()) => {
                warn!(
                    claim_id = %claim_id,
                    "Insight signup status poll returned an unexpected payload"
                );
                return Err(StatusCode::BAD_GATEWAY);
            }
        }
    }
}

pub(crate) async fn insight_gate() -> Result<(), StatusCode> {
    let (status, _) = crate::lts2_sys::get_lts_license_status_async().await;
    match status {
        LtsStatus::Invalid | LtsStatus::NotChecked => Err(StatusCode::FORBIDDEN),
        _ => Ok(()),
    }
}

pub(crate) async fn support_ticket_gate() -> Result<(), StatusCode> {
    let (status, _) = crate::lts2_sys::get_lts_license_status_async().await;
    match status {
        LtsStatus::AlwaysFree | LtsStatus::FreeTrial | LtsStatus::SelfHosted | LtsStatus::Full => {
            Ok(())
        }
        _ => Err(StatusCode::FORBIDDEN),
    }
}

pub async fn lts_trial_signup_data(license_key: String) -> Result<(), StatusCode> {
    info!("Received license key, enabling free trial: {}", license_key);
    if license_key == "FAIL" {
        warn!("Free trial request failed");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    info!("Free trial request succeeded, license key: {}", license_key);
    apply_insight_license(license_key, true).await
}

#[cfg(test)]
mod tests {
    use super::{
        SignupCheckResponse, SignupCheckState, build_insight_url, classify_signup_check_response,
        normalize_insight_base_url,
    };
    use uuid::Uuid;

    #[test]
    fn normalize_insight_base_url_defaults_to_hosted_insight() {
        assert_eq!(
            normalize_insight_base_url(None),
            "https://insight.libreqos.com/"
        );
    }

    #[test]
    fn normalize_insight_base_url_adds_scheme_and_trailing_slash() {
        assert_eq!(
            normalize_insight_base_url(Some("insight.example.com")),
            "https://insight.example.com/"
        );
    }

    #[test]
    fn normalize_insight_base_url_strips_signup_api_suffix() {
        assert_eq!(
            normalize_insight_base_url(Some("https://insight.example.com/signup-api/")),
            "https://insight.example.com/"
        );
    }

    #[test]
    fn build_insight_url_joins_paths_cleanly() {
        assert_eq!(
            build_insight_url(Some("insight.example.com"), "/su/start"),
            "https://insight.example.com/su/start"
        );
    }

    #[test]
    fn classify_signup_check_response_handles_pending() {
        let response = SignupCheckResponse {
            status: "pending".to_string(),
            account_key: None,
        };

        assert_eq!(
            classify_signup_check_response(response),
            Ok(SignupCheckState::Pending)
        );
    }

    #[test]
    fn classify_signup_check_response_requires_account_key_for_provisioned() {
        let response = SignupCheckResponse {
            status: "provisioned".to_string(),
            account_key: None,
        };

        assert!(classify_signup_check_response(response).is_err());
    }

    #[test]
    fn classify_signup_check_response_returns_provisioned_account_key() {
        let account_key = Uuid::new_v4();
        let response = SignupCheckResponse {
            status: "provisioned".to_string(),
            account_key: Some(account_key),
        };

        assert_eq!(
            classify_signup_check_response(response),
            Ok(SignupCheckState::Provisioned { account_key })
        );
    }
}
