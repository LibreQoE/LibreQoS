//! Handles license checks from the `license_server`.

use sqlx::{Pool, Postgres, Row};
use thiserror::Error;

pub async fn get_stats_host_for_key(cnn: Pool<Postgres>, key: &str) -> Result<String, StatsHostError> {
    let row = sqlx::query("SELECT ip_address FROM licenses INNER JOIN stats_hosts ON stats_hosts.id = licenses.stats_host WHERE key=$1")
        .bind(key)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let ip_address: &str = row.try_get("ip_address").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    log::info!("Found stats host for key: {}", ip_address);
    Ok(ip_address.to_string())
}

pub async fn insert_or_update_node_public_key(cnn: Pool<Postgres>, node_id: &str, node_name: &str, license_key: &str, public_key: &[u8]) -> Result<(), StatsHostError> {
    let row = sqlx::query("SELECT COUNT(*) AS count FROM shaper_nodes WHERE node_id=$1 AND license_key=$2")
        .bind(node_id)
        .bind(license_key)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let count: i64 = row.try_get("count").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    match count {
        0 => {
            // Insert
            log::info!("Inserting new node: {} {}", node_id, license_key);
            sqlx::query("INSERT INTO shaper_nodes (license_key, node_id, public_key, node_name) VALUES ($1, $2, $3, $4)")
                .bind(license_key)
                .bind(node_id)
                .bind(public_key)
                .bind(node_name)
                .execute(&cnn)
                .await
                .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        }
        1 => {
            // Update
            log::info!("Updating node: {} {}", node_id, license_key);
            sqlx::query("UPDATE shaper_nodes SET public_key=$1, last_seen=NOW(), node_name=$4 WHERE node_id=$2 AND license_key=$3")
                .bind(public_key)
                .bind(node_id)
                .bind(license_key)
                .bind(node_name)
                .execute(&cnn)
                .await
                .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        }
        _ => {
            log::error!("Found multiple nodes with the same node_id and license_key");
            return Err(StatsHostError::DatabaseError("Found multiple nodes with the same node_id and license_key".to_string()));
        }
    }

    Ok(())
}

pub async fn fetch_public_key(cnn: Pool<Postgres>, license_key: &str, node_id: &str) -> Result<Vec<u8>, StatsHostError> {
    let row = sqlx::query("SELECT public_key FROM shaper_nodes WHERE license_key=$1 AND node_id=$2")
        .bind(license_key)
        .bind(node_id)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let public_key: Vec<u8> = row.try_get("public_key").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    Ok(public_key)
}

#[derive(Debug, Error)]
pub enum StatsHostError {
    #[error("Database error occurred")]
    DatabaseError(String),
    #[error("Host already exists")]
    HostAlreadyExists,
    #[error("Organization already exists")]
    OrganizationAlreadyExists,
    #[error("No available stats hosts")]
    NoStatsHostsAvailable,
    #[error("InfluxDB Error")]
    InfluxError(String),
    #[error("No such login")]
    InvalidLogin,
}