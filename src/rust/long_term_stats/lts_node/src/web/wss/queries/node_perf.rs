use chrono::{DateTime, FixedOffset, Utc};
use influxdb2::FromDataPoint;
use pgdb::sqlx::{Pool, Postgres};
use tokio::sync::mpsc::Sender;
use wasm_pipe_types::{Perf, PerfHost, WasmResponse};
use super::{influx::InfluxTimePeriod, QueryBuilder};

#[derive(Debug, FromDataPoint)]
pub struct PerfRow {
    pub host_id: String,
    pub time: DateTime<FixedOffset>,
    pub cpu: f64,
    pub cpu_max: f64,
    pub ram: f64,
}

impl Default for PerfRow {
    fn default() -> Self {
        Self {
            host_id: "".to_string(),
            time: DateTime::<Utc>::MIN_UTC.into(),
            cpu: 0.0,
            cpu_max: 0.0,
            ram: 0.0,
        }
    }
}

pub async fn send_perf_for_node(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    period: InfluxTimePeriod,
    node_id: String,
    node_name: String,
) -> anyhow::Result<()> {
    let node = get_perf_for_node(cnn, key, node_id, node_name, &period).await?;
    tx.send(WasmResponse::NodePerfChart { nodes: vec![node] })
        .await?;
    Ok(())
}

pub async fn get_perf_for_node(
    cnn: &Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
    period: &InfluxTimePeriod,
) -> anyhow::Result<PerfHost> {
    let rows = QueryBuilder::new()
        .with_period(period)
        .derive_org(cnn, key)
        .await
        .bucket()
        .range()
        .measure_fields_org("perf", &["cpu", "cpu_max", "ram"])
        .filter(&format!("r[\"host_id\"] == \"{}\"", node_id))
        .aggregate_window()
        .execute::<PerfRow>()
        .await?;

    let mut stats = Vec::new();

    // Fill download
    for row in rows.iter() {
        stats.push(Perf {
            date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            cpu: row.cpu,
            cpu_max: row.cpu_max,
            ram: row.ram,
        });
    }

    Ok(PerfHost {
        node_id,
        node_name,
        stats,
    })
}
