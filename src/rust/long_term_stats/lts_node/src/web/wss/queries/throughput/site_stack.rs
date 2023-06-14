use crate::web::wss::{queries::time_period::InfluxTimePeriod, send_response};
use axum::extract::ws::WebSocket;
use pgdb::sqlx::{Pool, Postgres, Row};
use wasm_pipe_types::Throughput;

use super::{get_throughput_for_all_nodes_by_circuit, get_throughput_for_all_nodes_by_site};

pub async fn send_site_stack_map(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    period: InfluxTimePeriod,
    site_id: String,
) -> anyhow::Result<()> {
    let site_index = pgdb::get_site_id_from_name(cnn, key, &site_id).await?;
    //println!("Site index: {site_index}");

    let sites: Vec<String> =
        pgdb::sqlx::query("SELECT DISTINCT site_name FROM site_tree WHERE key=$1 AND parent=$2")
            .bind(key)
            .bind(site_index)
            .fetch_all(cnn)
            .await?
            .iter()
            .map(|row| row.try_get("site_name").unwrap())
            .collect();
    //println!("{sites:?}");

    let circuits: Vec<(String, String)> =
        pgdb::sqlx::query("SELECT DISTINCT circuit_id, circuit_name FROM shaped_devices WHERE key=$1 AND parent_node=$2")
            .bind(key)
            .bind(site_id)
            .fetch_all(cnn)
            .await?
            .iter()
            .map(|row| (row.try_get("circuit_id").unwrap(), row.try_get("circuit_name").unwrap()))
            .collect();
    //println!("{circuits:?}");

    let mut result = Vec::new();
    for site in sites.into_iter() {
        let mut throughput =
            get_throughput_for_all_nodes_by_site(cnn, key, period.clone(), &site).await?;
        throughput
            .iter_mut()
            .for_each(|row| row.node_name = site.clone());
        result.extend(throughput);
    }
    for circuit in circuits.into_iter() {
        let mut throughput =
            get_throughput_for_all_nodes_by_circuit(cnn, key, period.clone(), &circuit.0).await?;
        throughput
            .iter_mut()
            .for_each(|row| row.node_name = circuit.1.clone());
        result.extend(throughput);
    }
    //println!("{result:?}");

    // Sort by total
    result.sort_by(|a, b| {
        b.total()
            .partial_cmp(&a.total())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // If there are more than 9 entries, create an "others" to handle the remainder
    if result.len() > 9 {
        let mut others = wasm_pipe_types::ThroughputHost {
            node_id: "others".to_string(),
            node_name: "others".to_string(),
            down: Vec::new(),
            up: Vec::new(),
        };
        result[0].down.iter().for_each(|x| {
            others.down.push(Throughput {
                value: 0.0,
                date: x.date.clone(),
                l: 0.0,
                u: 0.0,
            });
        });
        result[0].up.iter().for_each(|x| {
            others.up.push(Throughput {
                value: 0.0,
                date: x.date.clone(),
                l: 0.0,
                u: 0.0,
            });
        });

        result.iter().skip(9).for_each(|row| {
            row.down.iter().enumerate().for_each(|(i, x)| {
                others.down[i].value += x.value;
            });
            row.up.iter().enumerate().for_each(|(i, x)| {
                others.up[i].value += x.value;
            });
        });

        result.truncate(9);
        result.push(others);
    }

    send_response(
        socket,
        wasm_pipe_types::WasmResponse::SiteStack { nodes: result },
    )
    .await;

    Ok(())
}
