mod per_node;
pub use per_node::*;
mod per_site;
pub use per_site::*;

use self::rtt_row::RttRow;
use super::{influx::InfluxTimePeriod, QueryBuilder};
use crate::web::wss::queries::rtt::rtt_row::RttCircuitRow;
use futures::future::join_all;
use influxdb2::{models::Query, Client};
use pgdb::{
    organization_cache::get_org_details,
    sqlx::{Pool, Postgres},
};
use tokio::sync::mpsc::Sender;
use tracing::instrument;
use wasm_pipe_types::{Rtt, RttHost, WasmResponse};
mod rtt_row;

#[instrument(skip(cnn, tx, key, site_id, period))]
pub async fn send_rtt_for_all_nodes_circuit(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    site_id: String,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    let nodes = get_rtt_for_all_nodes_circuit(cnn, key, &site_id, period).await?;

    let mut histogram = vec![0; 20];
    for node in nodes.iter() {
        for rtt in node.rtt.iter() {
            let bucket = usize::min(19, (rtt.value / 200.0) as usize);
            histogram[bucket] += 1;
        }
    }

    tx.send(WasmResponse::RttChartCircuit { nodes, histogram })
        .await?;
    Ok(())
}

pub async fn send_rtt_for_node(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    period: InfluxTimePeriod,
    node_id: String,
    node_name: String,
) -> anyhow::Result<()> {
    let node = get_rtt_for_node(cnn, key, node_id, node_name, &period).await?;
    let nodes = vec![node];

    tx.send(WasmResponse::RttChart { nodes }).await?;
    Ok(())
}

pub async fn get_rtt_for_all_nodes_circuit(
    cnn: &Pool<Postgres>,
    key: &str,
    circuit_id: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<Vec<RttHost>> {
    let node_status = pgdb::node_status(cnn, key).await?;
    let mut futures = Vec::new();
    for node in node_status {
        futures.push(get_rtt_for_node_circuit(
            cnn,
            key,
            node.node_id.to_string(),
            node.node_name.to_string(),
            circuit_id.to_string(),
            &period,
        ));
    }
    let all_nodes: anyhow::Result<Vec<RttHost>> = join_all(futures).await.into_iter().collect();
    all_nodes
}

pub async fn get_rtt_for_node(
    cnn: &Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
    period: &InfluxTimePeriod,
) -> anyhow::Result<RttHost> {
    let rows = QueryBuilder::new()
        .with_period(period)
        .derive_org(cnn, key)
        .await
        .bucket()
        .range()
        .measure_fields_org("rtt", &["avg", "min", "max"])
        .filter(&format!("r[\"host_id\"] == \"{}\"", node_id))
        .aggregate_window()
        .execute::<RttRow>()
        .await?;

    let mut rtt = Vec::new();

    // Fill RTT
    for row in rows.iter() {
        rtt.push(Rtt {
            value: f64::min(200.0, row.avg),
            date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            l: f64::min(200.0, row.min),
            u: f64::min(200.0, row.max) - f64::min(200.0, row.min),
        });
    }

    Ok(RttHost {
        node_id,
        node_name,
        rtt,
    })
}

pub async fn get_rtt_for_node_circuit(
    cnn: &Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
    circuit_id: String,
    period: &InfluxTimePeriod,
) -> anyhow::Result<RttHost> {
    let rows = QueryBuilder::new()
        .with_period(period)
        .derive_org(cnn, key)
        .await
        .bucket()
        .range()
        .measure_fields_org("rtt", &["avg", "min", "max"])
        .filter(&format!("r[\"host_id\"] == \"{}\"", node_id))
        .filter(&format!("r[\"circuit_id\"] == \"{}\"", circuit_id))
        .aggregate_window()
        .execute::<RttCircuitRow>()
        .await?;

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

    Ok(RttHost {
        node_id,
        node_name,
        rtt,
    })
}
