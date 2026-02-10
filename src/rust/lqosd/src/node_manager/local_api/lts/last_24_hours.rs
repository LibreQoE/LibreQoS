use crate::node_manager::shaper_queries_actor::ShaperQueryCommand;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::warn;

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

pub async fn throughput_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<ThroughputData>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query
        .send(ShaperQueryCommand::ShaperThroughput { seconds, reply: tx })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting total throughput: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(throughput)
}

pub async fn packets_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<FullPacketData>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query
        .send(ShaperQueryCommand::ShaperPackets { seconds, reply: tx })
        .await
        .map_err(|_| {
            warn!("Error sending packets period");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting packets period: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(throughput)
}

pub async fn percent_shaped_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<PercentShapedWeb>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query
        .send(ShaperQueryCommand::ShaperPercent { seconds, reply: tx })
        .await
        .map_err(|_| {
            warn!("Error sending percent shaped period");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting percent shaped: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(throughput)
}

pub async fn percent_flows_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<FlowCountViewWeb>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query
        .send(ShaperQueryCommand::ShaperFlows { seconds, reply: tx })
        .await
        .map_err(|_| {
            warn!("Error sending flows period");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting flows: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(throughput)
}

pub async fn rtt_histo_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<ShaperRttHistogramEntry>, StatusCode> {
    super::insight_gate().await?;
    tracing::error!("rtt_histo_period");
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query
        .send(ShaperQueryCommand::ShaperRttHistogram { seconds, reply: tx })
        .await
        .map_err(|_| {
            warn!("Error sending flows period");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting flows: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(throughput)
}

pub async fn top10_downloaders_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<Top10Circuit>, StatusCode> {
    super::insight_gate().await?;
    tracing::error!("rtt_histo_period");
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query
        .send(ShaperQueryCommand::ShaperTopDownloaders { seconds, reply: tx })
        .await
        .map_err(|_| {
            warn!("Error sending flows period");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting flows: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(throughput)
}

pub async fn worst10_rtt_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<Worst10RttCircuit>, StatusCode> {
    super::insight_gate().await?;
    tracing::error!("rtt_histo_period");
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query
        .send(ShaperQueryCommand::ShaperWorstRtt { seconds, reply: tx })
        .await
        .map_err(|_| {
            warn!("Error sending flows period");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting flows: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(throughput)
}

pub async fn worst10_rxmit_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<Worst10RxmitCircuit>, StatusCode> {
    super::insight_gate().await?;
    tracing::error!("rtt_histo_period");
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query
        .send(ShaperQueryCommand::ShaperWorstRxmit { seconds, reply: tx })
        .await
        .map_err(|_| {
            warn!("Error sending flows period");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting flows: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(throughput)
}

pub async fn top10_flows_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<AsnFlowSizeWeb>, StatusCode> {
    super::insight_gate().await?;
    tracing::error!("rtt_histo_period");
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query
        .send(ShaperQueryCommand::ShaperTopFlows { seconds, reply: tx })
        .await
        .map_err(|_| {
            warn!("Error sending flows period");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting flows: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(throughput)
}

pub async fn recent_medians_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
) -> Result<Vec<RecentMedians>, StatusCode> {
    super::insight_gate().await?;
    tracing::debug!("rtt_histo_period");
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query
        .send(ShaperQueryCommand::ShaperRecentMedian { reply: tx })
        .await
        .map_err(|_| {
            warn!("Error sending flows period");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let throughput = rx.await.map_err(|e| {
        warn!("Error getting flows: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(throughput)
}

pub async fn cake_period_data(
    shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>,
    seconds: i32,
) -> Result<Vec<CakeData>, StatusCode> {
    super::insight_gate().await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    shaper_query
        .send(ShaperQueryCommand::CakeTotals { seconds, reply: tx })
        .await
        .map_err(|_| {
            warn!("Error sending cake stats period");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let response = rx.await.map_err(|e| {
        warn!("Error getting cake stats: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(response)
}
