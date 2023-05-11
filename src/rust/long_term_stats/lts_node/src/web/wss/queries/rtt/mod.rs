use axum::extract::ws::{WebSocket, Message};
use futures::future::join_all;
use influxdb2::{Client, models::Query};
use pgdb::sqlx::{Pool, Postgres};
use crate::{submissions::get_org_details, web::wss::queries::rtt::rtt_row::RttCircuitRow};
use self::{rtt_row::{RttRow, RttSiteRow}, rtt_host::{Rtt, RttHost, RttChart}};

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

pub async fn send_rtt_for_all_nodes_site(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str, site_id: String, period: InfluxTimePeriod) -> anyhow::Result<()> {
    let nodes = get_rtt_for_all_nodes_site(cnn, key, &site_id, period).await?;

    let mut histogram = vec![0; 20];
    for node in nodes.iter() {
        for rtt in node.rtt.iter() {
            let bucket = usize::min(19, (rtt.value / 200.0) as usize);
            histogram[bucket] += 1;
        }
    }

    let chart = RttChart { msg: "rttChartSite".to_string(), nodes, histogram };
        let json = serde_json::to_string(&chart).unwrap();
        socket.send(Message::Text(json)).await.unwrap();
    Ok(())
}

pub async fn send_rtt_for_all_nodes_circuit(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str, site_id: String, period: InfluxTimePeriod) -> anyhow::Result<()> {
    let nodes = get_rtt_for_all_nodes_circuit(cnn, key, &site_id, period).await?;

    let mut histogram = vec![0; 20];
    for node in nodes.iter() {
        for rtt in node.rtt.iter() {
            let bucket = usize::min(19, (rtt.value / 200.0) as usize);
            histogram[bucket] += 1;
        }
    }

    let chart = RttChart { msg: "rttChartCircuit".to_string(), nodes, histogram };
        let json = serde_json::to_string(&chart).unwrap();
        socket.send(Message::Text(json)).await.unwrap();
    Ok(())
}

pub async fn send_rtt_for_node(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str, period: InfluxTimePeriod, node_id: String, node_name: String) -> anyhow::Result<()> {
    let node = get_rtt_for_node(cnn, key, node_id, node_name, period).await?;
    let nodes = vec![node];

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

pub async fn get_rtt_for_all_nodes_site(cnn: Pool<Postgres>, key: &str, site_id: &str, period: InfluxTimePeriod) -> anyhow::Result<Vec<RttHost>> {
    let node_status = pgdb::node_status(cnn.clone(), key).await?;
    let mut futures = Vec::new();
    for node in node_status {
        futures.push(get_rtt_for_node_site(
            cnn.clone(),
            key,
            node.node_id.to_string(),
            node.node_name.to_string(),
            site_id.to_string(),
            period.clone(),
        ));
    }
    let all_nodes: anyhow::Result<Vec<RttHost>> = join_all(futures).await
        .into_iter().collect();
    all_nodes
}

pub async fn get_rtt_for_all_nodes_circuit(cnn: Pool<Postgres>, key: &str, circuit_id: &str, period: InfluxTimePeriod) -> anyhow::Result<Vec<RttHost>> {
    let node_status = pgdb::node_status(cnn.clone(), key).await?;
    let mut futures = Vec::new();
    for node in node_status {
        futures.push(get_rtt_for_node_circuit(
            cnn.clone(),
            key,
            node.node_id.to_string(),
            node.node_name.to_string(),
            circuit_id.to_string(),
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

pub async fn get_rtt_for_node_site(
    cnn: Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
    site_id: String,
    period: InfluxTimePeriod,
) -> anyhow::Result<RttHost> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> {}
        |> filter(fn: (r) => r[\"_measurement\"] == \"tree\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"host_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"node_name\"] == \"{}\")
        |> filter(fn: (r) => r[\"_field\"] == \"rtt_avg\" or r[\"_field\"] == \"rtt_max\" or r[\"_field\"] == \"rtt_min\")
        |> {}
        |> yield(name: \"last\")",
            org.influx_bucket, period.range(), org.key, node_id, site_id, period.aggregate_window()
        );

        let query = Query::new(qs);
        let rows = client.query::<RttSiteRow>(Some(query)).await;
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
                        value: row.rtt_avg,
                        date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        l: row.rtt_min,
                        u: row.rtt_max - row.rtt_min,
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

pub async fn get_rtt_for_node_circuit(
    cnn: Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
    circuit_id: String,
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
        |> filter(fn: (r) => r[\"circuit_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"_field\"] == \"avg\" or r[\"_field\"] == \"max\" or r[\"_field\"] == \"min\")
        |> {}
        |> yield(name: \"last\")",
            org.influx_bucket, period.range(), org.key, node_id, circuit_id, period.aggregate_window()
        );
        //log::warn!("{qs}");
        let query = Query::new(qs);
        let rows = client.query::<RttCircuitRow>(Some(query)).await;
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
