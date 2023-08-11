//! Packet-per-second data queries
mod packet_row;
use self::packet_row::PacketRow;
use pgdb::sqlx::{Pool, Postgres};
use tokio::sync::mpsc::Sender;
use tracing::instrument;
use wasm_pipe_types::{PacketHost, Packets, WasmResponse};
use super::{influx::{InfluxTimePeriod, InfluxQueryBuilder}, QueryBuilder};

fn add_by_direction(direction: &str, down: &mut Vec<Packets>, up: &mut Vec<Packets>, row: &PacketRow) {
    match direction {
        "down" => {
            down.push(Packets {
                value: row.avg,
                date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                l: row.min,
                u: row.max - row.min,
            });
        }
        "up" => {
            up.push(Packets {
                value: row.avg,
                date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                l: row.min,
                u: row.max - row.min,
            });
        }
        _ => {}
    }
}

#[instrument(skip(cnn, tx, key, period))]
pub async fn send_packets_for_all_nodes(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    let node_status = pgdb::node_status(cnn, key).await?;
    let mut nodes = Vec::<PacketHost>::new();
    InfluxQueryBuilder::new(period.clone())
        .with_measurement("packets")
        .with_fields(&["min", "max", "avg"])
        .with_groups(&["host_id", "min", "max", "avg", "direction", "_field"])
        .execute::<PacketRow>(cnn, key)
        .await?
        .into_iter()
        .for_each(|row| {
            if let Some(node) = nodes.iter_mut().find(|n| n.node_id == row.host_id) {
                add_by_direction(&row.direction, &mut node.down, &mut node.up, &row);
            } else {
                let mut down = Vec::new();
                let mut up = Vec::new();

                add_by_direction(&row.direction, &mut down, &mut up, &row);

                let node_name = if let Some(node) = node_status.iter().find(|n| n.node_id == row.host_id) {
                    node.node_name.clone()
                } else {
                    row.host_id.clone()
                };

                nodes.push(PacketHost {
                    node_id: row.host_id,
                    node_name,
                    down,
                    up,
                });
            }
        });
    tx.send(wasm_pipe_types::WasmResponse::PacketChart { nodes }).await?;
    Ok(())
}

#[instrument(skip(cnn, tx, key, period))]
pub async fn send_packets_for_node(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    period: InfluxTimePeriod,
    node_id: &str,
    node_name: &str,
) -> anyhow::Result<()> {
    let node =
        get_packets_for_node(cnn, key, node_id.to_string(), node_name.to_string(), period).await?;

    tx.send(wasm_pipe_types::WasmResponse::PacketChart { nodes: vec![node] }).await?;
    Ok(())
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
    let rows = QueryBuilder::new()
        .with_period(&period)
        .derive_org(cnn, key)
        .await
        .bucket()
        .range()
        .measure_fields_org("packets", &["min", "max", "avg"])
        .with_host_id(&node_id)
        .aggregate_window()
        .execute::<PacketRow>()
        .await;



    match rows {
        Err(e) => {
            tracing::error!("Error querying InfluxDB (packets by node): {}", e);
            Err(anyhow::Error::msg("Unable to query influx"))
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

            Ok(PacketHost {
                node_id,
                node_name,
                down,
                up,
            })
        }
    }
}
