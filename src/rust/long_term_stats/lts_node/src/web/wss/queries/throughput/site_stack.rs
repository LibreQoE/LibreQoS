use crate::web::wss::queries::{influx::InfluxTimePeriod, QueryBuilder};
use pgdb::{
    organization_cache::get_org_details,
    sqlx::{Pool, Postgres},
    OrganizationDetails,
};
use tokio::sync::mpsc::Sender;
use tracing::{error, instrument};
use wasm_pipe_types::{SiteStackHost, WasmResponse};

#[derive(Debug, influxdb2::FromDataPoint)]
pub struct SiteStackRow {
    pub node_name: String,
    pub node_parents: String,
    pub bits_max: f64,
    pub time: chrono::DateTime<chrono::FixedOffset>,
    pub direction: String,
}

impl Default for SiteStackRow {
    fn default() -> Self {
        Self {
            node_name: "".to_string(),
            node_parents: "".to_string(),
            bits_max: 0.0,
            time: chrono::DateTime::<chrono::Utc>::MIN_UTC.into(),
            direction: "".to_string(),
        }
    }
}

#[derive(Debug, influxdb2::FromDataPoint)]
pub struct CircuitStackRow {
    pub circuit_id: String,
    pub max: f64,
    pub time: chrono::DateTime<chrono::FixedOffset>,
    pub direction: String,
}

impl Default for CircuitStackRow {
    fn default() -> Self {
        Self {
            circuit_id: "".to_string(),
            max: 0.0,
            time: chrono::DateTime::<chrono::Utc>::MIN_UTC.into(),
            direction: "".to_string(),
        }
    }
}

#[instrument(skip(cnn, tx, key, period))]
pub async fn send_site_stack_map(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    period: InfluxTimePeriod,
    site_id: String,
) -> anyhow::Result<()> {
    let site_index = pgdb::get_site_id_from_name(cnn, key, &site_id).await?;

    if let Some(org) = get_org_details(cnn, key).await {
        // Determine child hosts
        let hosts = pgdb::get_circuit_list_for_site(cnn, key, &site_id).await?;
        let host_filter = pgdb::circuit_list_to_influx_filter(&hosts);

        let (circuits, rows) = tokio::join!(
            query_circuits_influx(&org, &period, &hosts, &host_filter),
            query_site_stack_influx(&org, &period, site_index)
        );

        match rows {
            Err(e) => error!("Influxdb tree query error: {e}"),
            Ok(mut rows) => {
                if let Ok(circuits) = circuits {
                    rows.extend(circuits);
                }
                let mut result = site_rows_to_hosts(rows);
                reduce_to_x_entries(&mut result);

                // Send the reply
                tx.send(WasmResponse::SiteStack { nodes: result }).await?;
            }
        }
    }

    Ok(())
}

#[instrument(skip(org, period, hosts, host_filter))]
async fn query_circuits_influx(
    org: &OrganizationDetails,
    period: &InfluxTimePeriod,
    hosts: &[(String, String)],
    host_filter: &str,
) -> anyhow::Result<Vec<SiteStackRow>> {
    if host_filter.is_empty() {
        return Ok(Vec::new());
    }
    let rows = QueryBuilder::new()
        .with_period(period)
        .with_org(org.clone())
        .bucket()
        .range()
        .measure_field_org("host_bits", "max")
        .aggregate_window()
        .filter(host_filter)
        .group(&["circuit_id", "_field", "direction"])
        .execute::<CircuitStackRow>()
        .await?
        .into_iter()
        .map(|row| SiteStackRow {
            node_name: hosts
                .iter()
                .find(|h| h.0 == row.circuit_id)
                .unwrap()
                .1
                .clone(),
            node_parents: "".to_string(),
            bits_max: row.max / 8.0,
            time: row.time,
            direction: row.direction,
        })
        .collect();
    Ok(rows)
}

#[instrument(skip(org, period, site_index))]
async fn query_site_stack_influx(
    org: &OrganizationDetails,
    period: &InfluxTimePeriod,
    site_index: i32,
) -> anyhow::Result<Vec<SiteStackRow>> {
    Ok(QueryBuilder::new()
        .add_line("import \"strings\"")
        .with_period(period)
        .with_org(org.clone())
        .bucket()
        .range()
        .measure_field_org("tree", "bits_max")
        .filter_and(&["exists r[\"node_parents\"]", "exists r[\"node_index\"]"])
        .aggregate_window()
        .filter(&format!("strings.hasSuffix(v: r[\"node_parents\"], suffix: \"S{}S\" + r[\"node_index\"] + \"S\")", site_index))
        .group(&["node_name", "node_parents", "_field", "node_index", "direction"])
        .execute::<SiteStackRow>()
        .await?
    )
}

fn site_rows_to_hosts(rows: Vec<SiteStackRow>) -> Vec<SiteStackHost> {
    let mut result: Vec<SiteStackHost> = Vec::new();
    for row in rows.iter() {
        if let Some(r) = result.iter_mut().find(|r| r.node_name == row.node_name) {
            if row.direction == "down" {
                r.download.push((
                    row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                    row.bits_max as i64,
                ));
            } else {
                r.upload.push((
                    row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                    row.bits_max as i64,
                ));
            }
        } else if row.direction == "down" {
            result.push(SiteStackHost {
                node_name: row.node_name.clone(),
                download: vec![(
                    row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                    row.bits_max as i64,
                )],
                upload: vec![],
            });
        } else {
            result.push(SiteStackHost {
                node_name: row.node_name.clone(),
                upload: vec![(
                    row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                    row.bits_max as i64,
                )],
                download: vec![],
            });
        }
    }
    result
}

fn reduce_to_x_entries(result: &mut Vec<SiteStackHost>) {
    // Sort descending by total
    result.sort_by(|a, b| {
        b.total()
            .partial_cmp(&a.total())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    const MAX_HOSTS: usize = 8;
    if result.len() > MAX_HOSTS {
        let mut others = SiteStackHost {
            node_name: "others".to_string(),
            download: Vec::new(),
            upload: Vec::new(),
        };
        result[0].download.iter().for_each(|x| {
            others.download.push((x.0.clone(), 0));
            others.upload.push((x.0.clone(), 0));
        });
        result.iter().skip(MAX_HOSTS).for_each(|row| {
            row.download.iter().enumerate().for_each(|(i, x)| {
                if i < others.download.len() {
                    others.download[i].1 += x.1;
                }
            });
            row.upload.iter().enumerate().for_each(|(i, x)| {
                if i < others.upload.len() {
                    others.upload[i].1 += x.1;
                }
            });
        });
        result.truncate(MAX_HOSTS - 1);
        result.push(others);
    }
}
