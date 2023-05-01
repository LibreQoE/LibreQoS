use super::time_period::InfluxTimePeriod;
use crate::submissions::get_org_details;
use axum::extract::ws::{WebSocket, Message};
use chrono::{DateTime, FixedOffset, Utc};
use influxdb2::Client;
use influxdb2::{models::Query, FromDataPoint};
use pgdb::sqlx::{query, Pool, Postgres, Row};
use serde::Serialize;
use std::collections::HashMap;

pub async fn root_heat_map(
    cnn: Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    if let Some(org) = get_org_details(cnn.clone(), key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        // Get sites where parent=0 (for this setup)
        let hosts: Vec<String> = query("SELECT DISTINCT site_name FROM site_tree WHERE key=$1 AND parent=0 AND site_type='site'")
        .bind(key)
        .fetch_all(&cnn)
        .await?
        .iter()
        .map(|row| row.try_get("site_name").unwrap())
        .collect();

        let mut host_filter = "filter(fn: (r) => ".to_string();
        for host in hosts.iter() {
            host_filter += &format!("r[\"node_name\"] == \"{host}\" or ");
        }
        host_filter = host_filter[0..host_filter.len() - 4].to_string();
        host_filter += ")";

        // Query influx for RTT averages
        let qs = format!(
            "from(bucket: \"{}\")
        |> {}
        |> filter(fn: (r) => r[\"_measurement\"] == \"tree\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"_field\"] == \"rtt_avg\")
        |> {}
        |> {}
        |> yield(name: \"last\")",
            org.influx_bucket,
            period.range(),
            org.key,
            host_filter,
            period.aggregate_window()
        );
        println!("{qs}");

        let query = Query::new(qs);
        let rows = client.query::<HeatRow>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB: {}", e);
                return Err(anyhow::Error::msg("Unable to query influx"));
            }
            Ok(rows) => {
                let mut sorter: HashMap<String, Vec<(DateTime<FixedOffset>, f64)>> = HashMap::new();
                for row in rows.iter() {
                    if let Some(hat) = sorter.get_mut(&row.node_name) {
                        hat.push((row.time, row.rtt_avg));
                    } else {
                        sorter.insert(row.node_name.clone(), vec![(row.time, row.rtt_avg)]);
                    }
                }
                let msg = HeatMessage {
                    msg: "rootHeat".to_string(),
                    data: sorter,
                };
                let json = serde_json::to_string(&msg).unwrap();
                socket.send(Message::Text(json)).await.unwrap();
            }
        }
    }

    Ok(())
}

#[derive(Serialize)]
struct HeatMessage {
    msg: String,
    data: HashMap<String, Vec<(DateTime<FixedOffset>, f64)>>,
}

#[derive(Debug, FromDataPoint)]
pub struct HeatRow {
    pub node_name: String,
    pub rtt_avg: f64,
    pub time: DateTime<FixedOffset>,
}

impl Default for HeatRow {
    fn default() -> Self {
        Self {
            node_name: "".to_string(),
            rtt_avg: 0.0,
            time: DateTime::<Utc>::MIN_UTC.into(),
        }
    }
}
