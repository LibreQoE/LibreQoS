use sqlx::{Pool, Postgres, FromRow};

use crate::license::StatsHostError;

#[derive(Debug, FromRow)]
pub struct DeviceHit {
    pub circuit_id: String,
    pub circuit_name: String,
    pub score: f64,
}

#[derive(Debug, FromRow)]
pub struct SiteHit {
    pub site_name: String,
    pub site_type: String,
    pub score: f64,
}

pub async fn search_devices(
    cnn: &Pool<Postgres>,
    key: &str,
    term: &str,
) -> Result<Vec<DeviceHit>, StatsHostError> {

    const SQL: &str = "with input as (select $1 as q)
    select circuit_id, circuit_name, 1 - (input.q <<-> (circuit_name || ' ' || device_name || ' ' || mac)) as score
    from shaped_devices, input
    where 
    key = $2 AND
    (input.q <<-> (circuit_name || ' ' || device_name || ' ' || mac)) < 0.15
    order by input.q <<-> (circuit_name || ' ' || device_name || ' ' || mac)";

    let rows = sqlx::query_as::<_, DeviceHit>(SQL)
        .bind(term)
        .bind(key)
        .fetch_all(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()));

    if let Err(e) = &rows {
        log::error!("{e:?}");
    }

    rows
}

pub async fn search_ip(
    cnn: &Pool<Postgres>,
    key: &str,
    term: &str,
) -> Result<Vec<DeviceHit>, StatsHostError> {
    const SQL: &str = "with input as (select $1 as q)
    select shaped_device_ip.circuit_id AS circuit_id, 
        circuit_name || ' (' || shaped_device_ip.ip_range || '/' || shaped_device_ip.subnet || ')' AS circuit_name,
        1 - (input.q <<-> shaped_device_ip.ip_range) AS score
    FROM shaped_device_ip INNER JOIN shaped_devices 
        ON (shaped_devices.circuit_id = shaped_device_ip.circuit_id AND shaped_devices.key = shaped_device_ip.key), input
    WHERE shaped_device_ip.key = $2
    AND (input.q <<-> shaped_device_ip.ip_range) < 0.15
    ORDER BY (input.q <<-> shaped_device_ip.ip_range)";

    let rows = sqlx::query_as::<_, DeviceHit>(SQL)
        .bind(term)
        .bind(key)
        .fetch_all(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()));

    if let Err(e) = &rows {
        log::error!("{e:?}");
    }

    rows
}

pub async fn search_sites(
    cnn: &Pool<Postgres>,
    key: &str,
    term: &str,
) -> Result<Vec<SiteHit>, StatsHostError> {
    const SQL: &str = "with input as (select $1 as q)
    select site_name, site_type, 1 - (input.q <<-> site_name) as score
    from site_tree, input
    where 
    key = $2 AND
    (input.q <<-> site_name) < 0.15
    order by input.q <<-> site_name";

    let rows = sqlx::query_as::<_, SiteHit>(SQL)
        .bind(term)
        .bind(key)
        .fetch_all(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()));

    if let Err(e) = &rows {
        log::error!("{e:?}");
    }

    rows
}