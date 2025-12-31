use axum::http::StatusCode;
use lqos_config::load_config;
use serde::{Deserialize, Serialize};
use tracing::error;

#[derive(Debug, Serialize, Deserialize)]
pub struct ShaperStatus {
    name: String,
    last_seen_seconds_ago: f32,
}

pub async fn shaper_status_data() -> Result<Vec<ShaperStatus>, StatusCode> {
    super::insight_gate().await?;
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let url = format!(
        "https://{}/shaper_api/status",
        config
            .long_term_stats
            .clone()
            .lts_url
            .unwrap_or("insight.libreqos.com".to_string())
    );
    println!("URL: {}", url);

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| {
            error!("Error building reqwest client: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let shapers = client
        .get(&url)
        .header(
            "x-license-key",
            config
                .long_term_stats
                .clone()
                .license_key
                .unwrap_or("".to_string()),
        )
        .header("x-node-id", config.node_id.to_string())
        .send()
        .await
        .map_err(|e| {
            error!("Error getting shaper status: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .json::<Vec<ShaperStatus>>()
        .await
        .map_err(|e| {
            error!("Error parsing shaper status: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(shapers)
}
