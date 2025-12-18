use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse, ExecutiveSummaryHeader};
use lqos_utils::temporal_heatmap::{HeatmapBlocks, TemporalHeatmap};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::json;
use tokio::sync::mpsc::Sender;

static LAST_PUBLISH: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));
const MIN_INTERVAL: Duration = Duration::from_secs(1);

pub async fn executive_heatmaps(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::ExecutiveHeatmaps)
        .await
    {
        return;
    }

    if !should_publish() {
        return;
    }

    let circuits = fetch_circuit_heatmaps(bus_tx.clone())
        .await
        .unwrap_or_default();
    let sites = fetch_site_heatmaps(bus_tx.clone())
        .await
        .unwrap_or_default();
    let asns = fetch_asn_heatmaps(bus_tx.clone()).await.unwrap_or_default();
    let header = fetch_executive_header(bus_tx.clone())
        .await
        .unwrap_or_else(empty_header);
    let global = fetch_global_heatmap(bus_tx)
        .await
        .unwrap_or_else(empty_blocks);

    let payload = json!({
        "event": PublishedChannels::ExecutiveHeatmaps.to_string(),
        "data": {
            "header": header,
            "global": global,
            "circuits": circuits,
            "sites": sites,
            "asns": asns,
        }
    })
    .to_string();

    channels
        .send(PublishedChannels::ExecutiveHeatmaps, payload)
        .await;
}

fn should_publish() -> bool {
    let now = Instant::now();
    let mut lock = LAST_PUBLISH.lock();
    if let Some(last) = *lock {
        if now.duration_since(last) < MIN_INTERVAL {
            return false;
        }
    }
    *lock = Some(now);
    true
}

async fn fetch_circuit_heatmaps(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) -> Option<Vec<lqos_bus::CircuitHeatmapData>> {
    if let Some(BusResponse::CircuitHeatmaps(data)) =
        fetch_single_response(bus_tx, BusRequest::GetCircuitHeatmaps).await
    {
        return Some(data);
    }
    None
}

async fn fetch_site_heatmaps(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) -> Option<Vec<lqos_bus::SiteHeatmapData>> {
    if let Some(BusResponse::SiteHeatmaps(data)) =
        fetch_single_response(bus_tx, BusRequest::GetSiteHeatmaps).await
    {
        return Some(data);
    }
    None
}

async fn fetch_asn_heatmaps(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) -> Option<Vec<lqos_bus::AsnHeatmapData>> {
    if let Some(BusResponse::AsnHeatmaps(data)) =
        fetch_single_response(bus_tx, BusRequest::GetAsnHeatmaps).await
    {
        return Some(data);
    }
    None
}

async fn fetch_executive_header(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) -> Option<ExecutiveSummaryHeader> {
    if let Some(BusResponse::ExecutiveSummaryHeader(data)) =
        fetch_single_response(bus_tx, BusRequest::GetExecutiveSummaryHeader).await
    {
        return Some(data);
    }
    None
}

async fn fetch_global_heatmap(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
) -> Option<HeatmapBlocks> {
    if let Some(BusResponse::GlobalHeatmap(data)) =
        fetch_single_response(bus_tx, BusRequest::GetGlobalHeatmap).await
    {
        return Some(data);
    }
    None
}

async fn fetch_single_response(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>,
    request: BusRequest,
) -> Option<BusResponse> {
    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    bus_tx.send((tx, request)).await.ok()?;
    let replies = rx.await.ok()?;
    replies.responses.into_iter().next()
}

fn empty_blocks() -> HeatmapBlocks {
    TemporalHeatmap::new().blocks()
}

fn empty_header() -> ExecutiveSummaryHeader {
    ExecutiveSummaryHeader::default()
}
