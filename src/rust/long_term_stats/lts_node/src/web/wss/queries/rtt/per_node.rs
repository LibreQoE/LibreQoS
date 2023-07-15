use crate::web::wss::{queries::time_period::InfluxTimePeriod, send_response};
use axum::extract::ws::WebSocket;
use pgdb::{
    organization_cache::get_org_details,
    sqlx::{Pool, Postgres},
    NodeStatus, OrganizationDetails,
};
use tracing::{error, instrument};
use wasm_pipe_types::{Rtt, RttHost};

use super::rtt_row::RttRow;

#[instrument(skip(cnn, socket, key, period))]
pub async fn send_rtt_for_all_nodes(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    if let Some(org) = get_org_details(cnn, key).await {
        let result = query_rtt_all_nodes(&org, &period).await;
        match result {
            Err(e) => error!("Error querying InfluxDB for Per Node RTT: {e:?}"),
            Ok(result) => {
                let node_status = pgdb::node_status(cnn, key).await?;
                let nodes = rtt_rows_to_result(result, node_status);
                send_response(socket, wasm_pipe_types::WasmResponse::RttChart { nodes, histogram: Vec::new() }).await;
            }
        }
    }
    Ok(())
}

#[instrument(skip(org, period))]
async fn query_rtt_all_nodes(
    org: &OrganizationDetails,
    period: &InfluxTimePeriod,
) -> anyhow::Result<Vec<RttRow>> {
    let influx_url = format!("http://{}:8086", org.influx_host);
    let client = influxdb2::Client::new(influx_url, &org.influx_org, &org.influx_token);
    let qs = format!("from(bucket: \"{}\")
    |> {}
    |> filter(fn: (r) => r[\"_measurement\"] == \"rtt\")
    |> filter(fn: (r) => r[\"_field\"] == \"avg\" or r[\"_field\"] == \"min\" or r[\"_field\"] == \"max\")
    |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
    |> group(columns: [\"host_id\", \"_field\"])
    |> {}
    |> yield(name: \"last\")
    ", 
    org.influx_bucket, period.range(), org.key, period.aggregate_window()
    );
    //println!("{qs}");

    let query = influxdb2::models::Query::new(qs);
    Ok(client.query::<RttRow>(Some(query)).await?)
}

fn rtt_rows_to_result(rows: Vec<RttRow>, node_status: Vec<NodeStatus>) -> Vec<RttHost> {
    let mut result = Vec::<RttHost>::new();
    for row in rows.into_iter() {
        if let Some(host) = result.iter_mut().find(|h| h.node_id == row.host_id) {
            // We found one - add to it
            host.rtt.push(Rtt {
                value: f64::min(200.0, row.avg),
                date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                l: f64::min(200.0, row.min),
                u: f64::min(200.0, row.max) - f64::min(200.0, row.min),
            });
        } else {
            let rtt = vec![Rtt {
                value: f64::min(200.0, row.avg),
                date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                l: f64::min(200.0, row.min),
                u: f64::min(200.0, row.max) - f64::min(200.0, row.min),
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
