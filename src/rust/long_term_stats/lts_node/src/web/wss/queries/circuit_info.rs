use axum::extract::ws::{WebSocket, Message};
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;

#[derive(Serialize)]
struct CircuitInfoMessage {
    msg: String,
    data: Vec<CircuitList>,
}

#[derive(Serialize)]
pub struct CircuitList {
    pub circuit_name: String,
    pub device_id: String,
    pub device_name: String,
    pub parent_node: String,
    pub mac: String,
    pub download_min_mbps: i32,
    pub download_max_mbps: i32,
    pub upload_min_mbps: i32,
    pub upload_max_mbps: i32,
    pub comment: String,
    pub ip_range: String,
    pub subnet: i32,
}

impl From<pgdb::CircuitInfo> for CircuitList {
    fn from(circuit: pgdb::CircuitInfo) -> Self {
        Self {
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
}

pub async fn send_circuit_info(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str, circuit_id: &str) {
    if let Ok(hosts) = pgdb::get_circuit_info(cnn, key, circuit_id).await {
        let hosts = hosts.into_iter().map(CircuitList::from).collect::<Vec<_>>();
        let msg = CircuitInfoMessage {
            msg: "circuit_info".to_string(),
            data: hosts,
        };
        let json = serde_json::to_string(&msg).unwrap();
        if let Err(e) = socket.send(Message::Text(json)).await {
            tracing::error!("Error sending message: {}", e);
        }
    }
}