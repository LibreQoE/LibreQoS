use axum::extract::Path;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use lqos_config::load_config;
use crate::node_manager::local_api::lts::rest_client::lts_query;

#[derive(Serialize, Deserialize)]
pub struct ThroughputData {
    time: i64, // Unix timestamp
    max_down: i64,
    max_up: i64,
    min_down: i64,
    min_up: i64,
    median_down: i64,
    median_up: i64,
}

#[derive(Serialize, Deserialize)]
pub struct CakeData {
    time: i64, // Unix timestamp
    max_marks_down: i64,
    max_marks_up: i64,
    min_marks_down: i64,
    min_marks_up: i64,
    median_marks_down: i64,
    median_marks_up: i64,
    max_drops_down: i64,
    max_drops_up: i64,
    min_drops_down: i64,
    min_drops_up: i64,
    median_drops_down: i64,
    median_drops_up: i64,
}

pub async fn last_24_hours()-> Result<Json<Vec<ThroughputData>>, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let seconds = 24 * 60 * 60;
    let url = format!("https://{}/shaper_api/totalThroughput/{seconds}", config.long_term_stats.clone().lts_url.unwrap_or("stats.libreqos.io".to_string()));
    let throughput = lts_query(&url).await?;
    Ok(Json(throughput))
}

pub async fn throughput_period(Path(seconds): Path<i32>)-> Result<Json<Vec<ThroughputData>>, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let url = format!("https://{}/shaper_api/totalThroughput/{seconds}", config.long_term_stats.lts_url.clone().unwrap_or("stats.libreqos.io".to_string()));
    let throughput = lts_query(&url).await?;
    Ok(Json(throughput))
}

pub async fn retransmits_period(Path(seconds): Path<i32>)-> Result<Json<Vec<ThroughputData>>, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let url = format!("https://{}/shaper_api/totalRetransmits/{seconds}", config.long_term_stats.lts_url.clone().unwrap_or("stats.libreqos.io".to_string()));
    let throughput = lts_query(&url).await?;
    Ok(Json(throughput))
}

pub async fn cake_period(Path(seconds): Path<i32>)-> Result<Json<Vec<CakeData>>, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let url = format!("https://{}/shaper_api/totalCake/{seconds}", config.long_term_stats.lts_url.clone().unwrap_or("stats.libreqos.io".to_string()));
    let throughput = lts_query(&url).await?;
    Ok(Json(throughput))
}