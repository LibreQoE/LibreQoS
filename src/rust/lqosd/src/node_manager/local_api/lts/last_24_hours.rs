use axum::http::StatusCode;
use axum::Json;
use log::error;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use lqos_config::load_config;

#[derive(Serialize, Deserialize)]
pub struct ThroughputData {
    time: OffsetDateTime,
    max_down: i64,
    max_up: i64,
    min_down: i64,
    min_up: i64,
    median_down: i64,
    median_up: i64,
}

pub async fn last_24_hours()-> Result<Json<Vec<ThroughputData>>, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let url = format!("https://{}/shaper_api/totalThroughput", config.long_term_stats.lts_url.unwrap_or("stats.libreqos.io".to_string()));
    println!("URL: {}", url);

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| {
            error!("Error building reqwest client: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let throughput = client
        .get(&url)
        .header("x-license-key", config.long_term_stats.license_key.unwrap_or("".to_string()))
        .header("x-node-id", config.node_id.to_string())
        .send()
        .await
        .map_err(|e| {
            error!("Error getting throughput data: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .json::<Vec<ThroughputData>>()
        .await
        .map_err(|e| {
            error!("Error parsing throughput data: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(throughput))
}