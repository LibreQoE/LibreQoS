use crate::web::wss::{queries::time_period::InfluxTimePeriod, send_response};
use axum::extract::ws::WebSocket;
use pgdb::{sqlx::{Pool, Postgres, Row}, organization_cache::get_org_details};
use tracing::{instrument, error};
use wasm_pipe_types::{Throughput, SiteStackHost};
use super::{get_throughput_for_all_nodes_by_circuit, get_throughput_for_all_nodes_by_site};

#[derive(Debug, influxdb2::FromDataPoint)]
pub struct SiteStackRow {
    pub node_name: String,
    pub node_parents: String,
    pub bits_max: i64,
    pub time: chrono::DateTime<chrono::FixedOffset>,
}

impl Default for SiteStackRow {
    fn default() -> Self {
        Self {
            node_name: "".to_string(),
            node_parents: "".to_string(),
            bits_max: 0,
            time: chrono::DateTime::<chrono::Utc>::MIN_UTC.into(),
        }
    }
}

#[instrument(skip(cnn, socket, key, period))]
pub async fn send_site_stack_map(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    period: InfluxTimePeriod,
    site_id: String,
) -> anyhow::Result<()> {
    let site_index = pgdb::get_site_id_from_name(cnn, key, &site_id).await?;
    //println!("Site index: {site_index}");

    /*let sites: Vec<(i32, String)> =
        pgdb::sqlx::query("SELECT DISTINCT site_name, index FROM site_tree WHERE key=$1 AND parent=$2")
            .bind(key)
            .bind(site_index)
            .fetch_all(cnn)
            .await?
            .iter()
            .map(|row| (
                row.try_get("index").unwrap(),
                row.try_get("site_name").unwrap()),
            )
            .collect();*/
    //println!("{sites:?}");

    /*let circuits: Vec<(String, String)> =
        pgdb::sqlx::query("SELECT DISTINCT circuit_id, circuit_name FROM shaped_devices WHERE key=$1 AND parent_node=$2")
            .bind(key)
            .bind(site_id)
            .fetch_all(cnn)
            .await?
            .iter()
            .map(|row| (row.try_get("circuit_id").unwrap(), row.try_get("circuit_name").unwrap()))
            .collect();*/
    //println!("{circuits:?}");

    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = influxdb2::Client::new(influx_url, &org.influx_org, &org.influx_token);
        let qs = format!("import \"strings\"

        from(bucket: \"{}\")
        |> {}
        |> filter(fn: (r) => r[\"_field\"] == \"bits_max\")
        |> filter(fn: (r) => r[\"_measurement\"] == \"tree\")
        |> filter(fn: (r) => r[\"direction\"] == \"down\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => strings.hasSuffix(v: r[\"node_parents\"], suffix: \"S{}S\" + r[\"node_index\"] + \"S\" ))
        |> group(columns: [\"node_name\", \"node_parents\", \"_field\", \"node_index\"])
        |> {}
        |> yield(name: \"sum\")",
        org.influx_bucket, period.range(), org.key, site_index, period.aggregate_window_sum());

        //println!("{qs}");

        let query = influxdb2::models::Query::new(qs);
        //let rows = client.query_raw(Some(query)).await;
        let rows = client.query::<SiteStackRow>(Some(query)).await;
        match rows {
            Err(e) => error!("Influxdb tree query error: {e}"),
            Ok(rows) => {
                let mut result: Vec<SiteStackHost> = Vec::new();
                for row in rows.iter() {
                    if let Some(r) = result.iter_mut().find(|r| r.node_name == row.node_name) {
                        r.download.push((
                            row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                            row.bits_max
                        ));
                    } else {
                        result.push(SiteStackHost { 
                            node_name: row.node_name.clone(),
                            download: vec![(row.time.format("%Y-%m-%d %H:%M:%S").to_string(), row.bits_max)]
                        });
                    }
                }

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
                    };
                    result[0].download.iter().for_each(|x| {
                        others.download.push((x.0.clone(), 0));
                    });
                    result.iter().skip(MAX_HOSTS).for_each(|row| {
                        row.download.iter().enumerate().for_each(|(i, x)| {
                            others.download[i].1 += x.1;
                        });
                    });
                    result.truncate(MAX_HOSTS);
                    result.push(others);
                }

                // Send the reply
                send_response(
                    socket,
                    wasm_pipe_types::WasmResponse::SiteStack { nodes: result },
                )
                .await;
            }
        }
    }

    /*let mut result = Vec::new();
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
            get_throughput_for_all_nodes_by_circuit(cnn, key, period.clone(), &circuit.0).await?;
        throughput
            .iter_mut()
            .for_each(|row| row.node_name = circuit.1.clone());
        result.extend(throughput);
    }
    //println!("{result:?}");

    // Sort by total
    result.sort_by(|a, b| {
        b.total()
            .partial_cmp(&a.total())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // If there are more than 9 entries, create an "others" to handle the remainder
    if result.len() > 9 {
        let mut others = wasm_pipe_types::ThroughputHost {
            node_id: "others".to_string(),
            node_name: "others".to_string(),
            down: Vec::new(),
            up: Vec::new(),
        };
        result[0].down.iter().for_each(|x| {
            others.down.push(Throughput {
                value: 0.0,
                date: x.date.clone(),
                l: 0.0,
                u: 0.0,
            });
        });
        result[0].up.iter().for_each(|x| {
            others.up.push(Throughput {
                value: 0.0,
                date: x.date.clone(),
                l: 0.0,
                u: 0.0,
            });
        });

        result.iter().skip(9).for_each(|row| {
            row.down.iter().enumerate().for_each(|(i, x)| {
                others.down[i].value += x.value;
            });
            row.up.iter().enumerate().for_each(|(i, x)| {
                others.up[i].value += x.value;
            });
        });

        result.truncate(9);
        result.push(others);
    }

    send_response(
        socket,
        wasm_pipe_types::WasmResponse::SiteStack { nodes: result },
    )
    .await;*/

    Ok(())
}
