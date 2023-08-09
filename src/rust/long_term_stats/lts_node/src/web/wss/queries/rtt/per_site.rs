use crate::web::wss::{queries::time_period::InfluxTimePeriod, send_response, influx_query_builder::InfluxQueryBuilder};
use axum::extract::ws::WebSocket;
use pgdb::{
    sqlx::{Pool, Postgres},
    NodeStatus
};
use tracing::instrument;
use wasm_pipe_types::{Rtt, RttHost};
use super::rtt_row::RttSiteRow;

#[instrument(skip(cnn, socket, key, period))]
pub async fn send_rtt_for_all_nodes_site(
    cnn: &Pool<Postgres>, socket: &mut WebSocket, key: &str, site_name: String, period: InfluxTimePeriod
) -> anyhow::Result<()> {
    let rows = InfluxQueryBuilder::new(period.clone())
        .with_measurement("tree")
        .with_fields(&["rtt_avg", "rtt_min", "rtt_max"])
        .with_filter(format!("r[\"node_name\"] == \"{}\"", site_name))
        .with_groups(&["host_id", "_field"])
        .execute::<RttSiteRow>(cnn, key)
        .await?;
    let node_status = pgdb::node_status(cnn, key).await?;
    let nodes = rtt_rows_to_result(rows, node_status);
    send_response(socket, wasm_pipe_types::WasmResponse::RttChartSite { nodes }).await;

    Ok(())
}

fn rtt_rows_to_result(rows: Vec<RttSiteRow>, node_status: Vec<NodeStatus>) -> Vec<RttHost> {
    let mut result = Vec::<RttHost>::new();
    for row in rows.into_iter() {
        if let Some(host) = result.iter_mut().find(|h| h.node_id == row.host_id) {
            // We found one - add to it
            host.rtt.push(Rtt {
                value: f64::min(200.0, row.rtt_avg),
                date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                l: f64::min(200.0, row.rtt_min),
                u: f64::min(200.0, row.rtt_max) - f64::min(200.0, row.rtt_min),
            });
        } else {
            let rtt = vec![Rtt {
                value: f64::min(200.0, row.rtt_avg),
                date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                l: f64::min(200.0, row.rtt_min),
                u: f64::min(200.0, row.rtt_max) - f64::min(200.0, row.rtt_min),
            }];

            let node_name = node_status
                .iter()
                .filter(|n| n.node_id == row.host_id)
                .map(|n| n.node_name.clone())
                .next()
                .unwrap_or("".to_string());

            let new_host = RttHost {
                node_id: row.host_id,
                node_name,
                rtt,
            };
            result.push(new_host);
        }
    }
    result
}
