use axum::extract::Path;
use axum::http::StatusCode;
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use tracing::info;
use tracing::log::warn;
use lqos_config::load_config;
use crate::node_manager::local_api::lts::rest_client::lts_query;
use crate::node_manager::shaper_queries_actor::ShaperQueryCommand;

#[derive(Serialize, Deserialize, Copy, Clone)]
#[derive(Debug)]
pub struct ThroughputData {
    time: i64, // Unix timestamp
    max_down: i64,
    max_up: i64,
    min_down: i64,
    min_up: i64,
    median_down: i64,
    median_up: i64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FullPacketData {
    pub time: i64, // Unix timestamp
    pub max_down: i64,
    pub max_up: i64,
    pub max_tcp_down: i64,
    pub max_tcp_up: i64,
    pub max_udp_down: i64,
    pub max_udp_up: i64,
    pub max_icmp_down: i64,
    pub max_icmp_up: i64,
    pub min_down: i64,
    pub min_up: i64,
    pub min_tcp_down: i64,
    pub min_tcp_up: i64,
    pub min_udp_down: i64,
    pub min_udp_up: i64,
    pub min_icmp_down: i64,
    pub min_icmp_up: i64,
    pub median_down: i64,
    pub median_up: i64,
    pub median_tcp_down: i64,
    pub median_tcp_up: i64,
    pub median_udp_down: i64,
    pub median_udp_up: i64,
    pub median_icmp_down: i64,
    pub median_icmp_up: i64,
}

#[derive(Serialize, Deserialize)]
pub struct RetransmitData {
    time: i64, // Unix timestamp
    max_down: f64,
    max_up: f64,
    min_down: f64,
    min_up: f64,
    median_down: f64,
    median_up: f64,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PercentShapedWeb {
    pub time: i64,
    pub shaper_id: i64,
    pub percent_shaped: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FlowCountViewWeb {
    time: i64,
    shaper_id: i64,
    flow_count: f64,
}

pub async fn last_24_hours()-> Result<Json<Vec<ThroughputData>>, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let seconds = 24 * 60 * 60;
    let url = format!("https://{}/shaper_api/totalThroughput/{seconds}", config.long_term_stats.clone().lts_url.unwrap_or("insight.libreqos.com".to_string()));
    let throughput = lts_query(&url).await?;
    Ok(Json(throughput))
}

pub async fn throughput_period(
    Extension(shaper_query): Extension<tokio::sync::mpsc::Sender<ShaperQueryCommand>>,
    Path(seconds): Path<i32>,
)-> Result<Json<Vec<ThroughputData>>, StatusCode> {
    info!("Requesting throughput data for {} seconds", seconds);
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query.send(ShaperQueryCommand::ShaperThroughput { seconds, reply: tx }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    info!("Sent throughput request. Awaiting reply.");
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting total throughput: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    info!("Received throughput data.");
    Ok(Json(throughput))
}

pub async fn packets_period(
    Extension(shaper_query): Extension<tokio::sync::mpsc::Sender<ShaperQueryCommand>>,
    Path(seconds): Path<i32>,
)-> Result<Json<Vec<FullPacketData>>, StatusCode> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query.send(ShaperQueryCommand::ShaperPackets { seconds, reply: tx }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting total throughput: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(throughput))
}

pub async fn percent_shaped_period(
    Extension(shaper_query): Extension<tokio::sync::mpsc::Sender<ShaperQueryCommand>>,
    Path(seconds): Path<i32>,
)-> Result<Json<Vec<PercentShapedWeb>>, StatusCode> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query.send(ShaperQueryCommand::ShaperPercent { seconds, reply: tx }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting total throughput: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(throughput))
}

pub async fn percent_flows_period(
    Extension(shaper_query): Extension<tokio::sync::mpsc::Sender<ShaperQueryCommand>>,
    Path(seconds): Path<i32>,
)-> Result<Json<Vec<FlowCountViewWeb>>, StatusCode> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query.send(ShaperQueryCommand::ShaperFlows { seconds, reply: tx }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting total throughput: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(throughput))
}

pub async fn retransmits_period(Path(seconds): Path<i32>)-> Result<Json<Vec<RetransmitData>>, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let url = format!("https://{}/shaper_api/totalRetransmits/{seconds}", config.long_term_stats.lts_url.clone().unwrap_or("insight.libreqos.com".to_string()));
    let throughput = lts_query(&url).await?;
    Ok(Json(throughput))
}

pub async fn cake_period(Path(seconds): Path<i32>)-> Result<Json<Vec<CakeData>>, StatusCode> {
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let url = format!("https://{}/shaper_api/totalCake/{seconds}", config.long_term_stats.lts_url.clone().unwrap_or("insight.libreqos.com".to_string()));
    let throughput = lts_query(&url).await?;
    Ok(Json(throughput))
}