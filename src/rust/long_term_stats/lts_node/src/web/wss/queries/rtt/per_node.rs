use crate::web::wss::{queries::time_period::InfluxTimePeriod, influx_query_builder::InfluxQueryBuilder};
use pgdb::{
    sqlx::{Pool, Postgres},
    NodeStatus
};
use tokio::sync::mpsc::Sender;
use tracing::instrument;
use wasm_pipe_types::{Rtt, RttHost, WasmResponse};

use super::rtt_row::{RttRow, RttHistoRow};

#[instrument(skip(cnn, tx, key, period))]
pub async fn send_rtt_for_all_nodes(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    let rows = InfluxQueryBuilder::new(period.clone())
        .with_measurement("rtt")
        .with_fields(&["avg", "min", "max"])
        .with_groups(&["host_id", "_field"])
        .execute::<RttRow>(cnn, key)
        .await?;
    let node_status = pgdb::node_status(cnn, key).await?;
    let nodes = rtt_rows_to_result(rows, node_status);
    tx.send(WasmResponse::RttChart { nodes }).await?;

    Ok(())
}

#[instrument(skip(cnn, tx, key, period))]
pub async fn send_rtt_histogram_for_all_nodes(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    let rows = InfluxQueryBuilder::new(period.clone())
        .with_measurement("rtt")
        .with_field("avg")
        .sample_no_window()
        .execute::<RttHistoRow>(cnn, key)
        .await?;

    let mut histo = vec![0u32; 20];
    rows.iter().for_each(|row| {
        let rtt = f64::min(row.avg, 200.);
        let bucket = usize::min((rtt / 10.0) as usize, 19);
        histo[bucket] += 1;
    });

    tx.send(WasmResponse::RttHistogram { histogram: histo }).await?;

    Ok(())
}

pub(crate) fn rtt_rows_to_result(rows: Vec<RttRow>, node_status: Vec<NodeStatus>) -> Vec<RttHost> {
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
