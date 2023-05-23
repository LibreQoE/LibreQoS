use crate::web::wss::{queries::{
    time_period::InfluxTimePeriod,
}, send_response};
use axum::extract::ws::WebSocket;
use pgdb::sqlx::{Pool, Postgres, Row};

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
            get_throughput_for_all_nodes_by_circuit(cnn, key, period.clone(), &circuit.0)
                .await?;
        throughput
            .iter_mut()
            .for_each(|row| row.node_name = circuit.1.clone());
        result.extend(throughput);
    }
    //println!("{result:?}");

    send_response(socket, wasm_pipe_types::WasmResponse::SiteStack { nodes: result }).await;

    Ok(())
}
