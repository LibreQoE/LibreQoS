use axum::extract::ws::{WebSocket, Message};
use chrono::{DateTime, FixedOffset, Utc};
use influxdb2::{Client, FromDataPoint, models::Query};
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;
use crate::submissions::get_org_details;
use super::time_period::InfluxTimePeriod;

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

#[derive(Serialize, Debug)]
pub struct PerfHost {
    pub node_id: String,
    pub node_name: String,
    pub stats: Vec<Perf>,
}

#[derive(Serialize, Debug)]
pub struct Perf {
    pub date: String,
    pub cpu: f64,
    pub cpu_max: f64,
    pub ram: f64,
}

#[derive(Serialize, Debug)]
pub struct PacketChart {
    pub msg: String,
    pub nodes: Vec<PerfHost>,
}

pub async fn send_perf_for_node(
    cnn: Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    period: InfluxTimePeriod,
    node_id: String,
    node_name: String,
) -> anyhow::Result<()> {
    let node = get_perf_for_node(cnn, key, node_id, node_name, period).await?;

    let chart = PacketChart {
        msg: "nodePerfChart".to_string(),
        nodes: vec![node],
    };
    let json = serde_json::to_string(&chart).unwrap();
    socket.send(Message::Text(json)).await.unwrap();
    Ok(())
}

pub async fn get_perf_for_node(
    cnn: Pool<Postgres>,
    key: &str,
    node_id: String,
    node_name: String,
    period: InfluxTimePeriod,
) -> anyhow::Result<PerfHost> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> {}
        |> filter(fn: (r) => r[\"_measurement\"] == \"perf\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"host_id\"] == \"{}\")
        |> {}
        |> yield(name: \"last\")",
            org.influx_bucket, period.range(), org.key, node_id, period.aggregate_window()
        );

        let query = Query::new(qs);
        let rows = client.query::<PerfRow>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB: {}", e);
                return Err(anyhow::Error::msg("Unable to query influx"));
            }
            Ok(rows) => {
                // Parse and send the data
                //println!("{rows:?}");

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

                return Ok(PerfHost{
                    node_id,
                    node_name,
                    stats,
                });
            }
        }
    }
    Err(anyhow::Error::msg("Unable to query influx"))
}
