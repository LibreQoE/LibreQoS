use super::time_period::InfluxTimePeriod;
use crate::web::wss::influx_query_builder::InfluxQueryBuilder;
use crate::web::wss::send_response;
use axum::extract::ws::WebSocket;
use chrono::{DateTime, FixedOffset, Utc};
use influxdb2::FromDataPoint;
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;
use tracing::instrument;
use std::collections::HashMap;
use wasm_pipe_types::WasmResponse;
use itertools::Itertools;

fn headings_sorter<T: HeatMapData>(rows: Vec<T>) -> HashMap<String, Vec<(DateTime<FixedOffset>, f64)>> {
    let mut headings = rows.iter().map(|r| r.time()).collect::<Vec<_>>();
    headings.sort();
    let headings: Vec<DateTime<FixedOffset>> = headings.iter().dedup().cloned().collect();
    //println!("{headings:#?}");
    let defaults = headings.iter().map(|h| (*h, 0.0)).collect::<Vec<_>>();
    let mut sorter: HashMap<String, Vec<(DateTime<FixedOffset>, f64)>> = HashMap::new();
    for row in rows.into_iter() {
        let entry = sorter.entry(row.name()).or_insert(defaults.clone());
        if let Some(idx) = headings.iter().position(|h| h == &row.time()) {
            entry[idx] = (row.time(), row.avg());
        }
    }
    sorter
}

#[instrument(skip(cnn,socket,key,period))]
pub async fn root_heat_map(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    let rows: Vec<HeatRow> = InfluxQueryBuilder::new(period.clone())
        .with_import("strings")
        .with_measurement("tree")
        .with_fields(&["rtt_avg"])
        .sample_after_org()
        .with_filter("exists(r[\"node_parents\"])")
        .with_filter("strings.hasSuffix(suffix: \"S0S\" + r[\"node_index\"] + \"S\", v: r[\"node_parents\"])")
        .with_filter("r[\"_value\"] > 0.0")
        .with_groups(&["_field", "node_name"])
        .execute(cnn, key)
        .await?;

    let sorter = headings_sorter(rows);
    send_response(socket, WasmResponse::RootHeat { data: sorter }).await;

    Ok(())
}

#[instrument(skip(cnn, key, site_name, period))]
async fn site_circuits_heat_map(
    cnn: &Pool<Postgres>,
    key: &str,
    site_name: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<Vec<HeatCircuitRow>> {
    // List all hosts with this site as exact parent
    let hosts = pgdb::get_circuit_list_for_site(cnn, key, site_name).await?;
    let host_filter = pgdb::circuit_list_to_influx_filter(&hosts);

    let rows: Vec<HeatCircuitRow> = InfluxQueryBuilder::new(period.clone())
        .with_measurement("rtt")
        .with_fields(&["avg"])
        .sample_after_org()
        .with_filter(host_filter)
        .with_filter("r[\"_value\"] > 0.0")
        .with_groups(&["_field", "circuit_id"])
        .execute(cnn, key)
        .await?
        .into_iter()
        .map(|row: HeatCircuitRow| HeatCircuitRow {
            circuit_id: hosts.iter().find(|h| h.0 == row.circuit_id).unwrap().1.clone(),
            ..row
        })
        .collect();

    Ok(rows)
}

#[instrument(skip(cnn, socket, key, period))]
pub async fn site_heat_map(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    site_name: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {

    let (site_id, circuits) = tokio::join!(
        pgdb::get_site_id_from_name(cnn, key, site_name),
        site_circuits_heat_map(cnn, key, site_name, period.clone()),
    );
    let (site_id, circuits) = (site_id?, circuits?);

    let mut rows: Vec<HeatRow> = InfluxQueryBuilder::new(period.clone())
        .with_import("strings")
        .with_measurement("tree")
        .with_fields(&["rtt_avg"])
        .sample_after_org()
        .with_filter("exists(r[\"node_parents\"])")
        .with_filter(format!("strings.containsStr(substr: \"S{site_id}S\" + r[\"node_index\"] + \"S\", v: r[\"node_parents\"])"))
        .with_filter("r[\"_value\"] > 0.0")
        .with_groups(&["_field", "node_name"])
        .execute(cnn, key)
        .await?;

    circuits.iter().for_each(|c| {
        rows.push(HeatRow {
            node_name: c.circuit_id.clone(),
            rtt_avg: c.avg,
            time: c.time,
        })
    });

    let sorter = headings_sorter(rows);
    send_response(socket, WasmResponse::SiteHeat { data: sorter }).await;

    Ok(())
}

trait HeatMapData {
    fn avg(&self) -> f64;
    fn time(&self) -> DateTime<FixedOffset>;
    fn name(&self) -> String;
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

impl HeatMapData for HeatRow {
    fn avg(&self) -> f64 {
        self.rtt_avg
    }

    fn time(&self) -> DateTime<FixedOffset> {
        self.time
    }

    fn name(&self) -> String {
        self.node_name.clone()
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

impl HeatMapData for HeatCircuitRow {
    fn avg(&self) -> f64 {
        self.avg
    }

    fn time(&self) -> DateTime<FixedOffset> {
        self.time
    }

    fn name(&self) -> String {
        self.circuit_id.clone()
    }
}
