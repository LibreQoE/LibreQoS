use std::collections::HashSet;

use axum::extract::ws::WebSocket;
use pgdb::sqlx::{Pool, Postgres};
use wasm_pipe_types::CircuitList;

use crate::web::wss::send_response;

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

fn from(circuit: pgdb::CircuitInfo) -> CircuitList {
    CircuitList {
        circuit_name: circuit.circuit_name,
        device_id: circuit.device_id,
        device_name: circuit.device_name,
        parent_node: circuit.parent_node,
        mac: circuit.mac,
        download_min_mbps: circuit.download_min_mbps,
        download_max_mbps: circuit.download_max_mbps,
        upload_min_mbps: circuit.upload_min_mbps,
        upload_max_mbps: circuit.upload_max_mbps,
        comment: circuit.comment,
        ip_range: circuit.ip_range,
        subnet: circuit.subnet,
    }
}