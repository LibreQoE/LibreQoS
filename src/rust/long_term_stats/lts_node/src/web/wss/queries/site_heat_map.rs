use super::time_period::InfluxTimePeriod;
use crate::web::wss::send_response;
use axum::extract::ws::WebSocket;
use chrono::{DateTime, FixedOffset, Utc};
use influxdb2::Client;
use influxdb2::{models::Query, FromDataPoint};
use pgdb::organization_cache::get_org_details;
use pgdb::sqlx::{query, Pool, Postgres, Row};
use pgdb::OrganizationDetails;
use serde::Serialize;
use std::collections::HashMap;
use wasm_pipe_types::WasmResponse;

pub async fn root_heat_map(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        // Get sites where parent=0 (for this setup)
        let hosts: Vec<String> =
            query("SELECT DISTINCT site_name FROM site_tree WHERE key=$1 AND parent=0")
                .bind(key)
                .fetch_all(cnn)
                .await?
                .iter()
                .map(|row| row.try_get("site_name").unwrap())
                .filter(|row| row != "Root")
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
        //println!("{qs}");

        let query = Query::new(qs);
        let rows = client.query::<HeatRow>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB (root heat map): {}", e);
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
                send_response(socket, WasmResponse::RootHeat { data: sorter }).await;
            }
        }
    }

    Ok(())
}

async fn site_circuits_heat_map(
    cnn: &Pool<Postgres>,
    key: &str,
    site_name: &str,
    period: InfluxTimePeriod,
    sorter: &mut HashMap<String, Vec<(DateTime<FixedOffset>, f64)>>,
    client: Client,
    org: &OrganizationDetails,
) -> anyhow::Result<()> {
    // Get sites where parent=site_id (for this setup)
    let hosts: Vec<(String, String)> =
        query("SELECT DISTINCT circuit_id, circuit_name FROM shaped_devices WHERE key=$1 AND parent_node=$2")
            .bind(key)
            .bind(site_name)
            .fetch_all(cnn)
            .await?
            .iter()
            .map(|row| (row.try_get("circuit_id").unwrap(), row.try_get("circuit_name").unwrap()))
            .collect();

    let mut circuit_map = HashMap::new();
    for (id, name) in hosts.iter() {
        circuit_map.insert(id, name);
    }
    let hosts = hosts.iter().map(|(id, _)| id).collect::<Vec<_>>();

    let mut host_filter = "filter(fn: (r) => ".to_string();
    for host in hosts.iter() {
        host_filter += &format!("r[\"circuit_id\"] == \"{host}\" or ");
    }
    host_filter = host_filter[0..host_filter.len() - 4].to_string();
    host_filter += ")";

    // Query influx for RTT averages
    let qs = format!(
        "from(bucket: \"{}\")
    |> {}
    |> filter(fn: (r) => r[\"_measurement\"] == \"rtt\")
    |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
    |> filter(fn: (r) => r[\"_field\"] == \"avg\")
    |> {}
    |> {}
    |> yield(name: \"last\")",
        org.influx_bucket,
        period.range(),
        org.key,
        host_filter,
        period.aggregate_window()
    );
    //println!("{qs}\n\n");
    if qs.contains("filter(fn: (r))") {
        // No hosts to filter
        return Ok(());
    }

    let query = Query::new(qs);
    let rows = client.query::<HeatCircuitRow>(Some(query)).await;
    match rows {
        Err(e) => {
            tracing::error!("Error querying InfluxDB (site_circuits_heat_map): {}", e);
            return Err(anyhow::Error::msg("Unable to query influx"));
        }
        Ok(rows) => {
            for row in rows.iter() {
                if let Some(name) = circuit_map.get(&row.circuit_id) {
                    if let Some(hat) = sorter.get_mut(*name) {
                        hat.push((row.time, row.avg));
                    } else {
                        sorter.insert(name.to_string(), vec![(row.time, row.avg)]);
                    }
                }
            }
        }
    }

    Ok(())
}

pub async fn site_heat_map(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    site_name: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        // Get the site index
        let site_id = pgdb::get_site_id_from_name(cnn, key, site_name).await?;

        // Get sites where parent=site_id (for this setup)
        let hosts: Vec<String> =
            query("SELECT DISTINCT site_name FROM site_tree WHERE key=$1 AND parent=$2")
                .bind(key)
                .bind(site_id)
                .fetch_all(cnn)
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

        if host_filter.ends_with("(r))") {
            host_filter =
                "filter(fn: (r) => r[\"node_name\"] == \"bad_sheep_no_data\")".to_string();
        }

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
        //println!("{qs}\n\n");

        let query = Query::new(qs);
        let rows = client.query::<HeatRow>(Some(query)).await;
        match rows {
            Err(e) => {
                tracing::error!("Error querying InfluxDB (site-heat-map): {}", e);
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

                site_circuits_heat_map(cnn, key, site_name, period, &mut sorter, client, &org)
                    .await?;
                send_response(socket, WasmResponse::SiteHeat { data: sorter }).await;
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

#[derive(Debug, FromDataPoint)]
pub struct HeatCircuitRow {
    pub circuit_id: String,
    pub avg: f64,
    pub time: DateTime<FixedOffset>,
}

impl Default for HeatCircuitRow {
    fn default() -> Self {
        Self {
            circuit_id: "".to_string(),
            avg: 0.0,
            time: DateTime::<Utc>::MIN_UTC.into(),
        }
    }
}
