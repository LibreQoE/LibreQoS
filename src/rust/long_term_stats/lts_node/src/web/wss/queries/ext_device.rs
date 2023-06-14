use std::collections::HashSet;
use axum::extract::ws::WebSocket;
use chrono::{DateTime, FixedOffset};
use influxdb2::{FromDataPoint, models::Query, Client};
use pgdb::{sqlx::{Pool, Postgres}, organization_cache::get_org_details};

use crate::web::wss::send_response;

use super::time_period::InfluxTimePeriod;

pub async fn send_extended_device_info(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
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
            send_response(socket, wasm_pipe_types::WasmResponse::DeviceExt { data: extended_data }).await;
        }
    }
}

pub async fn send_extended_device_snr_graph(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    device_id: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> {}
        |> filter(fn: (r) => r[\"_measurement\"] == \"device_ext\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"device_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"_field\"] == \"noise_floor\" or r[\"_field\"] == \"rx_signal\")
        |> {}
        |> yield(name: \"last\")",
            org.influx_bucket, period.range(), org.key, device_id, period.aggregate_window()
        );
        //println!("{qs}");

        let query = Query::new(qs);
        let rows = client.query::<SnrRow>(Some(query)).await?;

        let mut sn = Vec::new();
        rows.iter().for_each(|row| {
            let snr = wasm_pipe_types::SignalNoiseChartExt {
                noise: row.noise_floor,
                signal: row.rx_signal,
                date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            };
            sn.push(snr);
        });
        send_response(socket, wasm_pipe_types::WasmResponse::DeviceExtSnr { data: sn, device_id: device_id.to_string() }).await;
    }
    Ok(())
}

#[derive(Debug, FromDataPoint, Default)]
pub struct SnrRow {
    pub device_id: String,
    pub noise_floor: f64,
    pub rx_signal: f64,
    pub time: DateTime<FixedOffset>,
}

pub async fn send_extended_device_capacity_graph(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    device_id: &str,
    period: InfluxTimePeriod,
) -> anyhow::Result<()> {
    if let Some(org) = get_org_details(cnn, key).await {
        let influx_url = format!("http://{}:8086", org.influx_host);
        let client = Client::new(influx_url, &org.influx_org, &org.influx_token);

        let qs = format!(
            "from(bucket: \"{}\")
        |> {}
        |> filter(fn: (r) => r[\"_measurement\"] == \"device_ext\")
        |> filter(fn: (r) => r[\"organization_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"device_id\"] == \"{}\")
        |> filter(fn: (r) => r[\"_field\"] == \"dl_capacity\" or r[\"_field\"] == \"ul_capacity\")
        |> {}
        |> yield(name: \"last\")",
            org.influx_bucket, period.range(), org.key, device_id, period.aggregate_window()
        );
        //println!("{qs}");

        let query = Query::new(qs);
        let rows = client.query::<CapacityRow>(Some(query)).await?;

        let mut sn = Vec::new();
        rows.iter().for_each(|row| {
            let snr = wasm_pipe_types::CapacityChartExt {
                dl: row.dl_capacity,
                ul: row.ul_capacity,
                date: row.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            };
            sn.push(snr);
        });
        send_response(socket, wasm_pipe_types::WasmResponse::DeviceExtCapacity { data: sn, device_id: device_id.to_string() }).await;
    }
    Ok(())
}

#[derive(Debug, FromDataPoint, Default)]
pub struct CapacityRow {
    pub device_id: String,
    pub dl_capacity: f64,
    pub ul_capacity: f64,
    pub time: DateTime<FixedOffset>,
}