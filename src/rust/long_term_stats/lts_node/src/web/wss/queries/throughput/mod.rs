use std::collections::HashMap;
mod site_stack;
use axum::extract::ws::WebSocket;
use futures::future::join_all;
use influxdb2::{Client, models::Query};
use pgdb::{sqlx::{Pool, Postgres}, organization_cache::get_org_details};
use wasm_pipe_types::{ThroughputHost, Throughput};
use crate::web::wss::send_response;
use self::throughput_row::{ThroughputRow, ThroughputRowBySite, ThroughputRowByCircuit};
use super::time_period::InfluxTimePeriod;
mod throughput_row;
pub use site_stack::send_site_stack_map;

pub async fn send_throughput_for_all_nodes(cnn: &Pool<Postgres>, socket: &mut WebSocket, key: &str, period: InfluxTimePeriod) -> anyhow::Result<()> {
    let nodes = get_throughput_for_all_nodes(cnn, key, period).await?;
    send_response(socket, wasm_pipe_types::WasmResponse::BitsChart { nodes }).await;
    Ok(())
}

pub async fn send_throughput_for_all_nodes_by_site(cnn: &Pool<Postgres>, socket: &mut WebSocket, key: &str, site_name: String, period: InfluxTimePeriod) -> anyhow::Result<()> {
    let nodes = get_throughput_for_all_nodes_by_site(cnn, key, period, &site_name).await?;

    send_response(socket, wasm_pipe_types::WasmResponse::BitsChart { nodes }).await;
    Ok(())
}

pub async fn send_throughput_for_all_nodes_by_circuit(cnn: &Pool<Postgres>, socket: &mut WebSocket, key: &str, circuit_id: String, period: InfluxTimePeriod) -> anyhow::Result<()> {
    let nodes = get_throughput_for_all_nodes_by_circuit(cnn, key, period, &circuit_id).await?;
    send_response(socket, wasm_pipe_types::WasmResponse::BitsChart { nodes }).await;
    Ok(())
}

pub async fn send_throughput_for_node(cnn: &Pool<Postgres>, socket: &mut WebSocket, key: &str, period: InfluxTimePeriod, node_id: String, node_name: String) -> anyhow::Result<()> {
    let node = get_throughput_for_node(cnn, key, node_id, node_name, period).await?;
    send_response(socket, wasm_pipe_types::WasmResponse::BitsChart { nodes: vec![node] }).await;
    Ok(())
}

pub async fn get_throughput_for_all_nodes(cnn: &Pool<Postgres>, key: &str, period: InfluxTimePeriod) -> anyhow::Result<Vec<ThroughputHost>> {
    let node_status = pgdb::node_status(cnn, key).await?;
    let mut futures = Vec::new();
    for node in node_status {
        futures.push(get_throughput_for_node(
            cnn,
            key,
            node.node_id.to_string(),
            node.node_name.to_string(),
            period.clone(),
        ));
    }
    let all_nodes: anyhow::Result<Vec<ThroughputHost>> = join_all(futures).await
        .into_iter().collect();
    all_nodes
}

pub async fn get_throughput_for_all_nodes_by_site(cnn: &Pool<Postgres>, key: &str, period: InfluxTimePeriod, site_name: &str) -> anyhow::Result<Vec<ThroughputHost>> {
    let node_status = pgdb::node_status(cnn, key).await?;
    let mut futures = Vec::new();
    for node in node_status {
        futures.push(get_throughput_for_node_by_site(
            cnn,
            key,
            node.node_id.to_string(),
            node.node_name.to_string(),
            site_name.to_string(),
            period.clone(),
        ));
    }
    let all_nodes: anyhow::Result<Vec<ThroughputHost>> = join_all(futures).await
        .into_iter().collect();
    all_nodes
}

pub async fn get_throughput_for_all_nodes_by_circuit(cnn: &Pool<Postgres>, key: &str, period: InfluxTimePeriod, circuit_id: &str) -> anyhow::Result<Vec<ThroughputHost>> {
    let node_status = pgdb::node_status(cnn, key).await?;
    let mut futures = Vec::new();
    for node in node_status {
        futures.push(get_throughput_for_node_by_circuit(
            cnn,
            key,
            node.node_id.to_string(),
            node.node_name.to_string(),
            circuit_id.to_string(),
            period.clone(),
        ));
    }
    let mut all_nodes = Vec::new();
    for node in (join_all(futures).await).into_iter().flatten() {
        all_nodes.extend(node);
    }
    Ok(all_nodes)
}

pub async fn get_throughput_for_node(
    cnn: &Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
    period: InfluxTimePeriod,
) -> anyhow::Result<ThroughputHost> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> {}
        |> filter(fn: (r) => r[\"_measurement\"] == \"bits\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"host_id\"] == \"{}\")
        |> {}
        |> yield(name: \"last\")",
            org.influx_bucket, period.range(), org.key, node_id, period.aggregate_window()
        );

        let query = Query::new(qs);
        let rows = client.query::<ThroughputRow>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB (throughput node): {}", e);
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
                        date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    });
                }

                // Fill upload
                for row in rows.iter().filter(|r| r.direction == "up") {
                    up.push(Throughput {
                        value: row.avg,
                        date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
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

pub async fn get_throughput_for_node_by_site(
    cnn: &Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
    site_name: String,
    period: InfluxTimePeriod,
) -> anyhow::Result<ThroughputHost> {
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
        |> filter(fn: (r) => r[\"_field\"] == \"bits_avg\" or r[\"_field\"] == \"bits_max\" or r[\"_field\"] == \"bits_min\")
        |> {}
        |> yield(name: \"last\")",
            org.influx_bucket, period.range(), org.key, node_id, site_name, period.aggregate_window()
        );

        let query = Query::new(qs);
        //println!("{:?}", query);
        let rows = client.query::<ThroughputRowBySite>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB (throughput site): {}", e);
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
                        value: row.bits_avg,
                        date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        l: row.bits_min,
                        u: row.bits_max - row.bits_min,
                    });
                }

                // Fill upload
                for row in rows.iter().filter(|r| r.direction == "up") {
                    up.push(Throughput {
                        value: row.bits_avg,
                        date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        l: row.bits_min,
                        u: row.bits_max - row.bits_min,
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

pub async fn get_throughput_for_node_by_circuit(
    cnn: &Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
    circuit_id: String,
    period: InfluxTimePeriod,
) -> anyhow::Result<Vec<ThroughputHost>> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> {}
        |> filter(fn: (r) => r[\"_measurement\"] == \"host_bits\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"host_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"circuit_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"_field\"] == \"avg\" or r[\"_field\"] == \"max\" or r[\"_field\"] == \"min\")
        |> {}
        |> yield(name: \"last\")",
            org.influx_bucket, period.range(), org.key, node_id, circuit_id, period.aggregate_window()
        );

        let query = Query::new(qs);
        //println!("{:?}", query);
        let rows = client.query::<ThroughputRowByCircuit>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!(" (throughput circuit): {}", e);
                return Err(anyhow::Error::msg("Unable to query influx"));
            }
            Ok(rows) => {
                // Parse and send the data
                //println!("{rows:?}");

                let mut sorter: HashMap<String, (Vec<Throughput>, Vec<Throughput>)> = HashMap::new();

                // Fill download
                for row in rows.iter().filter(|r| r.direction == "down") {
                    let tp = Throughput {
                        value: row.avg,
                        date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    };
                    if let Some(hat) = sorter.get_mut(&row.ip) {
                        hat.0.push(tp);
                    } else {
                        sorter.insert(row.ip.clone(), (vec![tp], Vec::new()));
                    }
                }

                // Fill upload
                for row in rows.iter().filter(|r| r.direction == "up") {
                    let tp = Throughput {
                        value: row.avg,
                        date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        l: row.min,
                        u: row.max - row.min,
                    };
                    if let Some(hat) = sorter.get_mut(&row.ip) {
                        hat.1.push(tp);
                    } else {
                        sorter.insert(row.ip.clone(), (Vec::new(), vec![tp]));
                    }
                }

                let mut result = Vec::new();

                for (ip, (down, up)) in sorter.iter() {
                    result.push(ThroughputHost{
                        node_id: node_id.clone(),
                        node_name: format!("{ip} {node_name}"),
                        down: down.clone(),
                        up: up.clone(),
                    });
                }

                return Ok(result);
            }
        }
    }
    Err(anyhow::Error::msg("Unable to query influx"))
}
