use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, FlowbeeSummaryData, LibreqosBusClient};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct TopAsnRow {
    pub name: String,
    pub value: u64,
    pub flow_count: u64,
    pub retransmit_percent: f64,
}

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
    entry.rx_down_num += f.tcp_retransmits.down as f64;
    entry.rx_down_den += f.packets_sent.down as f64;
    entry.rx_up_num += f.tcp_retransmits.up as f64;
    entry.rx_up_den += f.packets_sent.up as f64;
    entry.flows += 1;
}

async fn fetch_asn_top() -> Result<(Vec<TopAsnRow>, Vec<TopAsnRow>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::DumpActiveFlows])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;

    let mut map: HashMap<String, Agg> = HashMap::new();
    for response in responses {
        if let BusResponse::AllActiveFlows(flows) = response {
            for f in flows.into_iter() {
                accumulate(&mut map, f);
            }
        }
    }

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
    down_rows.sort_by_key(|row| std::cmp::Reverse(row.value));
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
    up_rows.sort_by_key(|row| std::cmp::Reverse(row.value));
    up_rows.truncate(9);

    Ok((down_rows, up_rows))
}

#[utoipa::path(
    get,
    path = "/asn_top_download",
    summary = "Top download ASNs by rate estimate",
    description = "Return the top download ASNs by estimated throughput rate.",
    responses(
        (status = 200, description = "Return 200 OK and top download ASNs.", body = Vec<TopAsnRow>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn asn_top_download() -> Result<(StatusCode, Json<Vec<TopAsnRow>>), (StatusCode, String)>
{
    let (down, _) = fetch_asn_top().await?;
    Ok((StatusCode::OK, Json(down)))
}

#[utoipa::path(
    get,
    path = "/asn_top_upload",
    summary = "Top upload ASNs by rate estimate",
    description = "Return the top upload ASNs by estimated throughput rate.",
    responses(
        (status = 200, description = "Return 200 OK and top upload ASNs.", body = Vec<TopAsnRow>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn asn_top_upload() -> Result<(StatusCode, Json<Vec<TopAsnRow>>), (StatusCode, String)> {
    let (_, up) = fetch_asn_top().await?;
    Ok((StatusCode::OK, Json(up)))
}
