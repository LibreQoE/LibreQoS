//! Packet-per-second data queries
mod packet_row;
use self::packet_row::PacketRow;
use super::time_period::InfluxTimePeriod;
use crate::web::wss::send_response;
use axum::extract::ws::WebSocket;
use futures::future::join_all;
use influxdb2::{models::Query, Client};
use pgdb::{sqlx::{Pool, Postgres}, organization_cache::get_org_details};
use wasm_pipe_types::{PacketHost, Packets};

pub async fn send_packets_for_all_nodes(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    let nodes = get_packets_for_all_nodes(cnn, key, period).await?;
    send_response(socket, wasm_pipe_types::WasmResponse::PacketChart { nodes }).await;
    Ok(())
}

pub async fn send_packets_for_node(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    period: InfluxTimePeriod,
    node_id: &str,
    node_name: &str,
) -> anyhow::Result<()> {
    let node =
        get_packets_for_node(cnn, key, node_id.to_string(), node_name.to_string(), period).await?;

    send_response(
        socket,
        wasm_pipe_types::WasmResponse::PacketChart { nodes: vec![node] },
    )
    .await;
    Ok(())
}

/// Requests packet-per-second data for all shaper nodes for a given organization
///
/// # Arguments
/// * `cnn` - A connection pool to the database
/// * `key` - The organization's license key
pub async fn get_packets_for_all_nodes(
    cnn: &Pool<Postgres>,
    key: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<Vec<PacketHost>> {
    let node_status = pgdb::node_status(cnn, key).await?;
    let mut futures = Vec::new();
    for node in node_status {
        futures.push(get_packets_for_node(
            cnn,
            key,
            node.node_id.to_string(),
            node.node_name.to_string(),
            period.clone(),
        ));
    }
    let all_nodes: anyhow::Result<Vec<PacketHost>> = join_all(futures).await.into_iter().collect();
    all_nodes
}

/// Requests packet-per-second data for a single shaper node.
///
/// # Arguments
/// * `cnn` - A connection pool to the database
/// * `key` - The organization's license key
/// * `node_id` - The ID of the node to query
/// * `node_name` - The name of the node to query
pub async fn get_packets_for_node(
    cnn: &Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
    period: InfluxTimePeriod,
) -> anyhow::Result<PacketHost> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> {}
        |> filter(fn: (r) => r[\"_measurement\"] == \"packets\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"host_id\"] == \"{}\")
        |> {}
        |> yield(name: \"last\")",
            org.influx_bucket,
            period.range(),
            org.key,
            node_id,
            period.aggregate_window()
        );

        let query = Query::new(qs);
        let rows = client.query::<PacketRow>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB (packets by node): {}", e);
                return Err(anyhow::Error::msg("Unable to query influx"));
            }
            Ok(rows) => {
                // Parse and send the data
                //println!("{rows:?}");

                let mut down = Vec::new();
                let mut up = Vec::new();

                // Fill download
                for row in rows.iter().filter(|r| r.direction == "down") {
                    down.push(Packets {
                        value: row.avg,
                        date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }

                // Fill upload
                for row in rows.iter().filter(|r| r.direction == "up") {
                    up.push(Packets {
                        value: row.avg,
                        date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }

                return Ok(PacketHost {
                    node_id,
                    node_name,
                    down,
                    up,
                });
            }
        }
    }
    Err(anyhow::Error::msg("Unable to query influx"))
}
