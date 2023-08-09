use pgdb::sqlx::{Pool, Postgres};
use tokio::sync::mpsc::Sender;
use wasm_pipe_types::{CircuitList, WasmResponse};

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

#[tracing::instrument(skip(cnn, tx, key, circuit_id))]
pub async fn send_circuit_info(cnn: &Pool<Postgres>, tx: Sender<WasmResponse>, key: &str, circuit_id: &str) {
    if let Ok(hosts) = pgdb::get_circuit_info(cnn, key, circuit_id).await {
        let hosts = hosts.into_iter().map(from).collect::<Vec<_>>();
        tx.send(WasmResponse::CircuitInfo { data: hosts }).await.unwrap();
    }
}