use axum::extract::ws::{WebSocket, Message};
use futures::future::join_all;
use influxdb2::{Client, models::Query};
use pgdb::sqlx::{Pool, Postgres};
use crate::submissions::get_org_details;
use self::{throughput_host::{ThroughputHost, Throughput, ThroughputChart}, throughput_row::ThroughputRow};
mod throughput_host;
mod throughput_row;

pub async fn send_throughput_for_all_nodes(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str) -> anyhow::Result<()> {
    let nodes = get_throughput_for_all_nodes(cnn, key).await?;

    let chart = ThroughputChart { msg: "bitsChart".to_string(), nodes };
        let json = serde_json::to_string(&chart).unwrap();
        socket.send(Message::Text(json)).await.unwrap();
    Ok(())
}

pub async fn get_throughput_for_all_nodes(cnn: Pool<Postgres>, key: &str) -> anyhow::Result<Vec<ThroughputHost>> {
    let node_status = pgdb::node_status(cnn.clone(), key).await?;
    let mut futures = Vec::new();
    for node in node_status {
        futures.push(get_throughput_for_node(
            cnn.clone(),
            key,
            node.node_id.to_string(),
            node.node_name.to_string(),
        ));
    }
    let all_nodes: anyhow::Result<Vec<ThroughputHost>> = join_all(futures).await
        .into_iter().collect();
    all_nodes
}

pub async fn get_throughput_for_node(
    cnn: Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
) -> anyhow::Result<ThroughputHost> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> range(start: -5m)
        |> filter(fn: (r) => r[\"_measurement\"] == \"bits\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"host_id\"] == \"{}\")
        |> aggregateWindow(every: 10s, fn: mean, createEmpty: false)
        |> yield(name: \"last\")",
            org.influx_bucket, org.key, node_id
        );

        let query = Query::new(qs);
        let rows = client.query::<ThroughputRow>(Some(query)).await;
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
                    down.push(Throughput {
                        value: row.avg,
                        date: row.time.format("%H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }

                // Fill upload
                for row in rows.iter().filter(|r| r.direction == "up") {
                    up.push(Throughput {
                        value: row.avg,
                        date: row.time.format("%H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }

                return Ok(ThroughputHost{
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
