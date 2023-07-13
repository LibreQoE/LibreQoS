use sqlx::{Pool, Postgres, Row};
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

pub async fn get_organization(cnn: &Pool<Postgres>, key: &str) -> Result<OrganizationDetails, StatsHostError> {
    let mut row = sqlx::query_as::<_, OrganizationDetails>("SELECT * FROM organizations WHERE key=$1")
        .bind(key)
        .fetch_one(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    
    // For local development - comment out
    if row.influx_host == "127.0.0.1" {
        row.influx_host = "146.190.156.69".to_string();
    }
    
    Ok(row)
}

pub async fn does_organization_name_exist(cnn: Pool<Postgres>, name: &str) -> Result<bool, StatsHostError> {
    let row = sqlx::query("SELECT COUNT(*) AS count FROM organizations WHERE name=$1")
        .bind(name)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    
    let count: i64 = row.try_get("count").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    Ok(count > 0)
}