use sqlx::{Pool, Postgres, FromRow};

use crate::license::StatsHostError;

#[derive(Debug, FromRow)]
pub struct DeviceHit {
    pub circuit_id: String,
    pub circuit_name: String,
    pub score: f64,
}

pub async fn search_devices(
    cnn: Pool<Postgres>,
    key: &str,
    term: &str,
) -> Result<Vec<DeviceHit>, StatsHostError> {

    const SQL: &str = "with input as (select $1 as q)
    select circuit_id, circuit_name, 1 - (input.q <<-> (circuit_name || ' ' || device_name || ' ' || mac_address || ' ' || ip_address)) as score
    from devices, input
    where 
    key = $2 AND
    input.q <% (circuit_name || ' ' || device_name || ' ' || mac_address || ' ' || ip_address)
    order by input.q <<-> (circuit_name || ' ' || device_name || ' ' || mac_address || ' ' || ip_address)";

    let rows = sqlx::query_as::<_, DeviceHit>(SQL)
        .bind(term)
        .bind(key)
        .fetch_all(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()));

    if let Err(e) = &rows {
        log::error!("{e:?}");
    }

    rows
}