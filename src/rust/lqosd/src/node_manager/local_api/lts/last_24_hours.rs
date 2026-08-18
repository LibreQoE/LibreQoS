use crate::node_manager::shaper_queries_actor::ShaperQueryCommand;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;
use tracing::warn;

const SHAPER_QUERY_RESPONSE_TIMEOUT: Duration = Duration::from_secs(35);
const SHAPER_QUERY_SEND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub struct ThroughputData {
    time: i64, // Unix timestamp
    max_down: i64,
    max_up: i64,
    min_down: i64,
    min_up: i64,
    median_down: i64,
    median_up: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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

#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FlowCountViewWeb {
    time: i64,
    shaper_id: i64,
    flow_count: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ShaperRttHistogramEntry {
    pub value: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Top10Circuit {
    pub shaper_id: i64,
    pub shaper_name: String,
    pub circuit_hash: String,
    pub circuit_name: String,
    pub bytes_down: f64,
    pub rtt: Option<f64>,
    pub rxmit: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Worst10RttCircuit {
    pub shaper_id: i64,
    pub shaper_name: String,
    pub circuit_hash: String,
    pub circuit_name: String,
    pub bytes_down: f64,
    pub rtt: Option<f64>,
    pub rxmit: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Worst10RxmitCircuit {
    pub shaper_id: i64,
    pub shaper_name: String,
    pub circuit_hash: String,
    pub circuit_name: String,
    pub bytes_down: f64,
    pub rtt: Option<f64>,
    pub rxmit: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AsnFlowSizeWeb {
    //pub start_time: i64, // Unix time
    pub shaper_id: i64,
    pub circuit_hash: i64,
    pub asn: i32,
    pub protocol: String,
    pub bytes_down: i64,
    pub bytes_up: i64,
    pub rtt_down: f32,
    pub rtt_up: f32,
    pub rxmit_down: f32,
    pub rxmit_up: f32,
    pub circuit_name: Option<String>,
    pub asn_name: Option<String>,
    pub shaper_name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RecentMedians {
    pub yesterday: (i64, i64),
    pub last_week: (i64, i64),
}

async fn wait_for_shaper_response<T>(
    context: &'static str,
    rx: tokio::sync::oneshot::Receiver<Vec<T>>,
) -> Result<Vec<T>, StatusCode> {
    match timeout(SHAPER_QUERY_RESPONSE_TIMEOUT, rx).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(err)) => {
            warn!("Error getting {context}: {err:?}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Err(_) => {
            warn!("Timed out waiting for {context}");
            Err(StatusCode::GATEWAY_TIMEOUT)
        }
    }
}

async fn send_shaper_query(
    context: &'static str,
    shaper_query: &tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    command: ShaperQueryCommand,
) -> Result<(), StatusCode> {
    match timeout(SHAPER_QUERY_SEND_TIMEOUT, shaper_query.send(command)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => {
            warn!("Error sending {context} query");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Err(_) => {
            warn!("Timed out queueing {context} query");
            Err(StatusCode::GATEWAY_TIMEOUT)
        }
    }
}

pub async fn throughput_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<ThroughputData>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_shaper_query(
        "throughput",
        &shaper_query,
        ShaperQueryCommand::ShaperThroughput { seconds, reply: tx },
    )
    .await?;
    let throughput = wait_for_shaper_response("total throughput", rx).await?;
    Ok(throughput)
}

pub async fn packets_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<FullPacketData>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_shaper_query(
        "packets period",
        &shaper_query,
        ShaperQueryCommand::ShaperPackets { seconds, reply: tx },
    )
    .await?;
    let throughput = wait_for_shaper_response("packets period", rx).await?;
    Ok(throughput)
}

pub async fn percent_shaped_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<PercentShapedWeb>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_shaper_query(
        "percent shaped period",
        &shaper_query,
        ShaperQueryCommand::ShaperPercent { seconds, reply: tx },
    )
    .await?;
    let throughput = wait_for_shaper_response("percent shaped", rx).await?;
    Ok(throughput)
}

pub async fn percent_flows_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<FlowCountViewWeb>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_shaper_query(
        "flows period",
        &shaper_query,
        ShaperQueryCommand::ShaperFlows { seconds, reply: tx },
    )
    .await?;
    let throughput = wait_for_shaper_response("flows", rx).await?;
    Ok(throughput)
}

pub async fn rtt_histo_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<ShaperRttHistogramEntry>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_shaper_query(
        "RTT histogram period",
        &shaper_query,
        ShaperQueryCommand::ShaperRttHistogram { seconds, reply: tx },
    )
    .await?;
    let throughput = wait_for_shaper_response("RTT histogram", rx).await?;
    Ok(throughput)
}

pub async fn top10_downloaders_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<Top10Circuit>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_shaper_query(
        "top downloaders period",
        &shaper_query,
        ShaperQueryCommand::ShaperTopDownloaders { seconds, reply: tx },
    )
    .await?;
    let throughput = wait_for_shaper_response("top downloaders", rx).await?;
    Ok(throughput)
}

pub async fn worst10_rtt_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<Worst10RttCircuit>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_shaper_query(
        "worst RTT period",
        &shaper_query,
        ShaperQueryCommand::ShaperWorstRtt { seconds, reply: tx },
    )
    .await?;
    let throughput = wait_for_shaper_response("worst RTT", rx).await?;
    Ok(throughput)
}

pub async fn worst10_rxmit_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<Worst10RxmitCircuit>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_shaper_query(
        "worst retransmits period",
        &shaper_query,
        ShaperQueryCommand::ShaperWorstRxmit { seconds, reply: tx },
    )
    .await?;
    let throughput = wait_for_shaper_response("worst retransmits", rx).await?;
    Ok(throughput)
}

pub async fn top10_flows_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<AsnFlowSizeWeb>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_shaper_query(
        "top flows period",
        &shaper_query,
        ShaperQueryCommand::ShaperTopFlows { seconds, reply: tx },
    )
    .await?;
    let throughput = wait_for_shaper_response("top flows", rx).await?;
    Ok(throughput)
}

pub async fn recent_medians_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
) -> Result<Vec<RecentMedians>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_shaper_query(
        "recent median",
        &shaper_query,
        ShaperQueryCommand::ShaperRecentMedian { reply: tx },
    )
    .await?;
    let throughput = wait_for_shaper_response("recent medians", rx).await?;
    Ok(throughput)
}

pub async fn cake_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<CakeData>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    send_shaper_query(
        "cake stats period",
        &shaper_query,
        ShaperQueryCommand::CakeTotals { seconds, reply: tx },
    )
    .await?;
    let response = wait_for_shaper_response("cake stats", rx).await?;
    Ok(response)
}
