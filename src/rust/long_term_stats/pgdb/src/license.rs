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

#[derive(Debug, Error)]
pub enum StatsHostError {
    #[error("Database error occurred")]
    DatabaseError(String),
}