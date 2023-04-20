use sqlx::{Pool, Postgres, Row};
use crate::license::StatsHostError;

pub async fn add_stats_host(cnn: Pool<Postgres>, hostname: String, influx_host: String, api_key: String) -> Result<i64, StatsHostError> {
    // Does the stats host already exist? We don't want duplicates
    let row = sqlx::query("SELECT COUNT(*) AS count FROM stats_hosts WHERE ip_address=$1")
        .bind(&hostname)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let count: i64 = row.try_get("count").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    if count != 0 {
        return Err(StatsHostError::HostAlreadyExists);
    }

    // Get the new primary key
    log::info!("Getting new primary key for stats host");
    let row = sqlx::query("SELECT NEXTVAL('stats_hosts_id_seq') AS id")
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        

    let new_id: i64 = row.try_get("id").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    // Insert the stats host
    log::info!("Inserting new stats host: {} ({})", hostname, new_id);
    sqlx::query("INSERT INTO stats_hosts (id, ip_address, can_accept_new_clients, influx_host, api_key) VALUES ($1, $2, $3, $4, $5)")
        .bind(new_id)
        .bind(&hostname)
        .bind(true)
        .bind(&influx_host)
        .bind(&api_key)
        .execute(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;   

    Ok(new_id)
}

const FIND_STATS_HOST: &str = "SELECT a.id AS id, a.influx_host AS influx_host, a.api_key AS api_key
FROM stats_hosts a
WHERE can_accept_new_clients = true
ORDER BY (SELECT COUNT(organizations.\"key\") FROM organizations WHERE a.influx_host = influx_host)
LIMIT 1";

pub async fn find_emptiest_stats_host(cnn: Pool<Postgres>) -> Result<(i32, String, String), StatsHostError> {
    let row = sqlx::query(FIND_STATS_HOST)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let id: i32 = row.try_get("id").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    let influx_host: String = row.try_get("influx_host").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    let api_key: String = row.try_get("api_key").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    Ok((id, influx_host, api_key))
}