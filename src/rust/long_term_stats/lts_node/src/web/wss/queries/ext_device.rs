use std::collections::HashSet;
use axum::extract::ws::WebSocket;
use chrono::{DateTime, FixedOffset};
use influxdb2::FromDataPoint;
use pgdb::sqlx::{Pool, Postgres};
use tokio::sync::mpsc::Sender;
use wasm_pipe_types::{WasmResponse, SignalNoiseChartExt, CapacityChartExt};
use super::{influx::InfluxTimePeriod, QueryBuilder};

#[tracing::instrument(skip(cnn, tx, key, circuit_id))]
pub async fn send_extended_device_info(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    circuit_id: &str,
) {
    // Get devices for circuit
    if let Ok(hosts_list) = pgdb::get_circuit_info(cnn, key, circuit_id).await {
        // Get the hosts known to be in this circuit
        let mut hosts = HashSet::new();
        hosts_list.into_iter().for_each(|h| {
            hosts.insert(h.device_id);
        });
        if hosts.is_empty() {
            return;
        }
        println!("{hosts:?}");

        // Get extended data
        let mut extended_data = Vec::new();
        for host in hosts.iter() {
            let ext = pgdb::get_device_info_ext(cnn, key, host).await;
            if let Ok(ext) = ext {
                let mut ext_wasm = wasm_pipe_types::ExtendedDeviceInfo {
                    device_id: ext.device_id.clone(),
                    name: ext.name.clone(),
                    model: ext.model.clone(),
                    firmware: ext.firmware.clone(),
                    status: ext.status.clone(),
                    mode: ext.mode.clone(),
                    channel_width: ext.channel_width,
                    tx_power: ext.tx_power,
                    interfaces: Vec::new(),
                };
                if let Ok(interfaces) = pgdb::get_device_interfaces_ext(cnn, key, host).await {
                    for ed in interfaces {
                        let edw = wasm_pipe_types::ExtendedDeviceInterface {
                            name: ed.name,
                            mac: ed.mac,
                            status: ed.status,
                            speed: ed.speed,
                            ip_list: ed.ip_list,
                        };
                        ext_wasm.interfaces.push(edw);
                    }
                }
                extended_data.push(ext_wasm);
            } else {
                tracing::error!("Error getting extended device info: {:?}", ext);
            }
        }
        // If there is any, send it
        println!("{extended_data:?}");
        if !extended_data.is_empty() {
            tx.send(WasmResponse::DeviceExt { data: extended_data }).await.unwrap();
        }
    }
}

#[tracing::instrument(skip(cnn, tx, key, device_id, period))]
pub async fn send_extended_device_snr_graph(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    device_id: &str,
    period: &InfluxTimePeriod,
) -> anyhow::Result<()> {
    let rows = QueryBuilder::new()
        .with_period(period)
        .derive_org(cnn, key)
        .await
        .bucket()
        .range()
        .measure_fields_org("device_ext", &["noise_floor", "rx_signal"])
        .filter(&format!("r[\"device_id\"] == \"{}\"", device_id))
        .aggregate_window()
        .execute::<SnrRow>()
        .await?
        .into_iter()
        .map(|row| {
            wasm_pipe_types::SignalNoiseChartExt {
                noise: row.noise_floor,
                signal: row.rx_signal,
                date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            }
        })
        .collect::<Vec<SignalNoiseChartExt>>();
    tx.send(WasmResponse::DeviceExtSnr { data: rows, device_id: device_id.to_string() }).await?;
    Ok(())
}

pub async fn send_ap_snr(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    site_name: &str,
    period: InfluxTimePeriod,

) -> anyhow::Result<()> {
    // Get list of child devices
    let hosts = pgdb::get_host_list_for_site(cnn, key, site_name).await?;
    let host_filter = pgdb::device_list_to_influx_filter(&hosts);

    Ok(())
}

#[derive(Debug, FromDataPoint, Default)]
pub struct SnrRow {
    pub device_id: String,
    pub noise_floor: f64,
    pub rx_signal: f64,
    pub time: DateTime<FixedOffset>,
}

#[tracing::instrument(skip(cnn, tx, key, device_id, period))]
pub async fn send_extended_device_capacity_graph(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    device_id: &str,
    period: &InfluxTimePeriod,
) -> anyhow::Result<()> {
    let rows = QueryBuilder::new()
        .with_period(period)
        .derive_org(cnn, key)
        .await
        .bucket()
        .range()
        .measure_fields_org("device_ext", &["dl_capacity", "ul_capacity"])
        .filter(&format!("r[\"device_id\"] == \"{}\"", device_id))
        .aggregate_window()
        .execute::<CapacityRow>()
        .await?
        .into_iter()
        .map(|row| {
            wasm_pipe_types::CapacityChartExt {
                dl: row.dl_capacity,
                ul: row.ul_capacity,
                date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            }
        })
        .collect::<Vec<CapacityChartExt>>();
    tx.send(WasmResponse::DeviceExtCapacity { data: rows, device_id: device_id.to_string() }).await?;
    Ok(())
}

#[derive(Debug, FromDataPoint, Default)]
pub struct CapacityRow {
    pub device_id: String,
    pub dl_capacity: f64,
    pub ul_capacity: f64,
    pub time: DateTime<FixedOffset>,
}