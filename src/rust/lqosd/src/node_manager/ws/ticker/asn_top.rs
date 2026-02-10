use std::collections::HashMap;
use std::sync::Arc;

use crate::node_manager::ws::messages::{TopAsnRow, WsResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusReply, BusRequest, BusResponse, FlowbeeSummaryData};
use tokio::sync::mpsc::Sender;

#[derive(Default)]
struct Agg {
    name: String,
    sum_down: u64,
    sum_up: u64,
    rx_down_num: f64,
    rx_down_den: f64,
    rx_up_num: f64,
    rx_up_den: f64,
    flows: u64,
}

pub async fn asn_top(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    // Only work if at least one of the channels is alive
    let live_down = channels
        .is_channel_alive(PublishedChannels::AsnTopDownload)
        .await;
    let live_up = channels
        .is_channel_alive(PublishedChannels::AsnTopUpload)
        .await;
    if !live_down && !live_up {
        return;
    }

    // Query all active flows from the bus
    let (tx, rx) = tokio::sync::oneshot::channel::<BusReply>();
    if let Err(e) = bus_tx.send((tx, BusRequest::DumpActiveFlows)).await {
        tracing::warn!("AsnTop: failed to send request to bus: {:?}", e);
        return;
    }
    let replies = match rx.await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("AsnTop: failed to receive reply from bus: {:?}", e);
            return;
        }
    };

    // Aggregate by ASN name
    let mut map: HashMap<String, Agg> = HashMap::new();
    for reply in replies.responses.into_iter() {
        if let BusResponse::AllActiveFlows(flows) = reply {
            for f in flows.into_iter() {
                accumulate(&mut map, f);
            }
        }
    }

    // Build sorted top lists
    let mut down_rows: Vec<TopAsnRow> = map
        .values()
        .map(|a| TopAsnRow {
            name: a.name.clone(),
            value: a.sum_down,
            flow_count: a.flows,
            retransmit_percent: if a.rx_down_den > 0.0 {
                (a.rx_down_num / a.rx_down_den) * 100.0
            } else {
                0.0
            },
        })
        .collect();
    down_rows.sort_by(|a, b| b.value.cmp(&a.value));
    down_rows.truncate(9);

    let mut up_rows: Vec<TopAsnRow> = map
        .values()
        .map(|a| TopAsnRow {
            name: a.name.clone(),
            value: a.sum_up,
            flow_count: a.flows,
            retransmit_percent: if a.rx_up_den > 0.0 {
                (a.rx_up_num / a.rx_up_den) * 100.0
            } else {
                0.0
            },
        })
        .collect();
    up_rows.sort_by(|a, b| b.value.cmp(&a.value));
    up_rows.truncate(9);

    // Publish to channels as needed
    if live_down {
        let message = WsResponse::AsnTopDownload { data: down_rows };
        channels
            .send(PublishedChannels::AsnTopDownload, message)
            .await;
    }
    if live_up {
        let message = WsResponse::AsnTopUpload { data: up_rows };
        channels
            .send(PublishedChannels::AsnTopUpload, message)
            .await;
    }
}

fn accumulate(map: &mut HashMap<String, Agg>, f: FlowbeeSummaryData) {
    let key = if !f.remote_asn_name.is_empty() {
        f.remote_asn_name.clone()
    } else {
        f.remote_ip.clone()
    };
    let entry = map.entry(key.clone()).or_insert_with(|| Agg {
        name: key,
        ..Default::default()
    });
    entry.sum_down = entry
        .sum_down
        .saturating_add(f.rate_estimate_bps.down as u64);
    entry.sum_up = entry.sum_up.saturating_add(f.rate_estimate_bps.up as u64);
    // Accumulate rxmit numerators/denominators for proper ratio
    entry.rx_down_num += f.tcp_retransmits.down as f64;
    entry.rx_down_den += f.packets_sent.down as f64;
    entry.rx_up_num += f.tcp_retransmits.up as f64;
    entry.rx_up_den += f.packets_sent.up as f64;
    entry.flows += 1;
}
