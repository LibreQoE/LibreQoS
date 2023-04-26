use axum::extract::ws::{WebSocket, Message};
use futures::future::join_all;
use influxdb2::{Client, models::Query};
use pgdb::sqlx::{Pool, Postgres};
use crate::submissions::get_org_details;
use self::{rtt_row::RttRow, rtt_host::{Rtt, RttHost, RttChart}};

use super::time_period::InfluxTimePeriod;
mod rtt_row;
mod rtt_host;

pub async fn send_rtt_for_all_nodes(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str, period: InfluxTimePeriod) -> anyhow::Result<()> {
    let nodes = get_rtt_for_all_nodes(cnn, key, period).await?;

    let mut histogram = vec![0; 20];
    for node in nodes.iter() {
        for rtt in node.rtt.iter() {
            let bucket = usize::min(19, (rtt.value / 200.0) as usize);
            histogram[bucket] += 1;
        }
    }

    let chart = RttChart { msg: "rttChart".to_string(), nodes, histogram };
        let json = serde_json::to_string(&chart).unwrap();
        socket.send(Message::Text(json)).await.unwrap();
    Ok(())
}

pub async fn get_rtt_for_all_nodes(cnn: Pool<Postgres>, key: &str, period: InfluxTimePeriod) -> anyhow::Result<Vec<RttHost>> {
    let node_status = pgdb::node_status(cnn.clone(), key).await?;
    let mut futures = Vec::new();
    for node in node_status {
        futures.push(get_rtt_for_node(
            cnn.clone(),
            key,
            node.node_id.to_string(),
            node.node_name.to_string(),
            period.clone(),
        ));
    }
    let all_nodes: anyhow::Result<Vec<RttHost>> = join_all(futures).await
        .into_iter().collect();
    all_nodes
}

pub async fn get_rtt_for_node(
    cnn: Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
    period: InfluxTimePeriod,
) -> anyhow::Result<RttHost> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> {}
        |> filter(fn: (r) => r[\"_measurement\"] == \"rtt\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"host_id\"] == \"{}\")
        |> {}
        |> yield(name: \"last\")",
            org.influx_bucket, period.range(), org.key, node_id, period.aggregate_window()
        );

        let query = Query::new(qs);
        let rows = client.query::<RttRow>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB: {}", e);
                return Err(anyhow::Error::msg("Unable to query influx"));
            }
            Ok(rows) => {
                // Parse and send the data
                //println!("{rows:?}");

                let mut rtt = Vec::new();

                // Fill download
                for row in rows.iter() {
                    rtt.push(Rtt {
                        value: row.avg,
                        date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }

                return Ok(RttHost{
                    node_id,
                    node_name,
                    rtt,
                });
            }
        }
    }
    Err(anyhow::Error::msg("Unable to query influx"))
}
