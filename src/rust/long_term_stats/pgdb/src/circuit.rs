use sqlx::{Pool, Postgres, FromRow};

use crate::license::StatsHostError;

#[derive(Debug, FromRow)]
pub struct CircuitInfo {
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

pub async fn get_circuit_info(
    cnn: &Pool<Postgres>,
    key: &str,
    circuit_id: &str,
) -> Result<Vec<CircuitInfo>, StatsHostError> {
    const SQL: &str = "SELECT circuit_name, device_id, device_name, parent_node, mac, download_min_mbps, download_max_mbps, upload_min_mbps, upload_max_mbps, comment, ip_range, subnet FROM shaped_devices INNER JOIN shaped_device_ip ON shaped_device_ip.key = shaped_devices.key AND shaped_device_ip.circuit_id = shaped_devices.circuit_id WHERE shaped_devices.key=$1 AND shaped_devices.circuit_id=$2";

    sqlx::query_as::<_, CircuitInfo>(SQL)
        .bind(key)
        .bind(circuit_id)
        .fetch_all(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))
}