//! Packet-per-second data queries
mod packet_host;
mod packet_row;
use self::{packet_host::{Packets, PacketHost, PacketChart}, packet_row::PacketRow};
use crate::submissions::get_org_details;
use axum::extract::ws::{WebSocket, Message};
use futures::future::join_all;
use influxdb2::{models::Query, Client};
use pgdb::sqlx::{Pool, Postgres};

pub async fn send_packets_for_all_nodes(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str) -> anyhow::Result<()> {
    let nodes = get_packets_for_all_nodes(cnn, key).await?;

    let chart = PacketChart { msg: "packetChart".to_string(), nodes };
        let json = serde_json::to_string(&chart).unwrap();
        socket.send(Message::Text(json)).await.unwrap();
    Ok(())
}

/// Requests packet-per-second data for all shaper nodes for a given organization
///
/// # Arguments
/// * `cnn` - A connection pool to the database
/// * `key` - The organization's license key
pub async fn get_packets_for_all_nodes(cnn: Pool<Postgres>, key: &str) -> anyhow::Result<Vec<PacketHost>> {
    let node_status = pgdb::node_status(cnn.clone(), key).await?;
    let mut futures = Vec::new();
    for node in node_status {
        futures.push(get_packets_for_node(
            cnn.clone(),
            key,
            node.node_id.to_string(),
            node.node_name.to_string(),
        ));
    }
    let all_nodes: anyhow::Result<Vec<PacketHost>> = join_all(futures).await
        .into_iter().collect();
    Ok(all_nodes?)
}

/// Requests packet-per-second data for a single shaper node.
/// 
/// # Arguments
/// * `cnn` - A connection pool to the database
/// * `key` - The organization's license key
/// * `node_id` - The ID of the node to query
/// * `node_name` - The name of the node to query
pub async fn get_packets_for_node(
    cnn: Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
) -> anyhow::Result<PacketHost> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> range(start: -5m)
        |> filter(fn: (r) => r[\"_measurement\"] == \"packets\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"host_id\"] == \"{}\")
        |> aggregateWindow(every: 10s, fn: mean, createEmpty: false)
        |> yield(name: \"last\")",
            org.influx_bucket, org.key, node_id
        );

        let query = Query::new(qs);
        let rows = client.query::<PacketRow>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB: {}", e);
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
                        date: row.time.format("%H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }

                // Fill upload
                for row in rows.iter().filter(|r| r.direction == "up") {
                    up.push(Packets {
                        value: row.avg,
                        date: row.time.format("%H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }

                return Ok(PacketHost{
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
