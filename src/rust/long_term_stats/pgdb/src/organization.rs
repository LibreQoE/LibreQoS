use sqlx::{Pool, Postgres};
use crate::license::StatsHostError;

#[derive(Clone, sqlx::FromRow, Debug)]
pub struct OrganizationDetails {
    pub key: String,
    pub name: String,
    pub influx_host: String,
    pub influx_org: String,
    pub influx_token: String,
    pub influx_bucket: String,
}

pub async fn get_organization(cnn: Pool<Postgres>, key: &str) -> Result<OrganizationDetails, StatsHostError> {
    let row = sqlx::query_as::<_, OrganizationDetails>("SELECT * FROM organizations WHERE key=$1")
        .bind(key)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    Ok(row)
}