use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::node_manager::ws::messages::{ExecutiveHeatmapsData, OversubscribedSite, WsResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES};
use crate::throughput_tracker::THROUGHPUT_TRACKER;
use lqos_bus::{BusReply, BusRequest, BusResponse, ExecutiveSummaryHeader};
use lqos_utils::temporal_heatmap::{HeatmapBlocks, TemporalHeatmap};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
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
    let global_qoq = THROUGHPUT_TRACKER.global_qoq_heatmap.lock().blocks();
    let oversubscribed_sites = compute_oversubscribed_sites();

    let payload = WsResponse::ExecutiveHeatmaps {
        data: ExecutiveHeatmapsData {
            header,
            global,
            global_qoq,
            circuits,
            sites,
            asns,
            oversubscribed_sites,
        },
    };

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

#[derive(Default)]
struct OversubTally {
    cap_down: f32,
    cap_up: f32,
    sub_down: f32,
    sub_up: f32,
}

fn compute_oversubscribed_sites() -> Vec<OversubscribedSite> {
    let devices = SHAPED_DEVICES.load();
    let mut circuit_map: HashMap<String, (String, f32, f32)> = HashMap::new();
    for device in devices.devices.iter() {
        let entry = circuit_map
            .entry(device.circuit_id.clone())
            .or_insert_with(|| (device.parent_node.clone(), 0.0_f32, 0.0_f32));
        // Keep the largest per-circuit subscription values.
        entry.1 = entry.1.max(device.download_max_mbps);
        entry.2 = entry.2.max(device.upload_max_mbps);
    }

    let reader = NETWORK_JSON.read();
    let nodes = reader.get_nodes_when_ready();
    if nodes.is_empty() {
        return Vec::new();
    }

    // Build children adjacency from immediate_parent.
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (idx, node) in nodes.iter().enumerate() {
        if let Some(parent) = node.immediate_parent {
            if parent < children.len() {
                children[parent].push(idx);
            }
        }
    }

    // Helper to collect descendant names including the node itself.
    let collect_descendants = |start: usize, children: &Vec<Vec<usize>>, nodes: &Vec<lqos_config::NetworkJsonNode>| {
        let mut stack = vec![start];
        let mut visited = HashSet::new();
        let mut names = HashSet::new();
        while let Some(idx) = stack.pop() {
            if !visited.insert(idx) {
                continue;
            }
            if let Some(name) = nodes.get(idx).map(|n| n.name.clone()) {
                names.insert(name);
            }
            if let Some(kids) = children.get(idx) {
                for &child in kids {
                    stack.push(child);
                }
            }
        }
        names
    };

    let mut results: Vec<(String, OversubTally)> = Vec::new();
    for (idx, node) in nodes.iter().enumerate() {
        if node.name == "Root" {
            continue;
        }
        let node_type = node
            .node_type
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase();
        if node_type != "site" && !node_type.is_empty() && node_type != "ap" {
            continue;
        }

        let mut tally = OversubTally::default();
        tally.cap_down = node.max_throughput.0 as f32;
        tally.cap_up = node.max_throughput.1 as f32;

        let descendants = collect_descendants(idx, &children, nodes);
        for (_circuit_id, (parent_name, down, up)) in circuit_map.iter() {
            if descendants.contains(parent_name) {
                tally.sub_down += *down;
                tally.sub_up += *up;
            }
        }

        results.push((node.name.clone(), tally));
    }

    results
        .into_iter()
        .filter_map(|(site_name, t)| {
            let ratio_down = if t.cap_down > 0.0 {
                Some(t.sub_down / t.cap_down)
            } else {
                None
            };
            let ratio_up = if t.cap_up > 0.0 {
                Some(t.sub_up / t.cap_up)
            } else {
                None
            };
            if ratio_down.is_none() && ratio_up.is_none() {
                return None;
            }
            let ratio_max = match (ratio_down, ratio_up) {
                (Some(d), Some(u)) => Some(d.max(u)),
                (Some(d), None) => Some(d),
                (None, Some(u)) => Some(u),
                (None, None) => None,
            };
            Some(OversubscribedSite {
                site_name,
                cap_down: t.cap_down,
                cap_up: t.cap_up,
                sub_down: t.sub_down,
                sub_up: t.sub_up,
                ratio_down,
                ratio_up,
                ratio_max,
            })
        })
        .collect()
}
