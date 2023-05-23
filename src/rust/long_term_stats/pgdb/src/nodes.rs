use sqlx::{Pool, Postgres};

use crate::license::StatsHostError;

pub async fn new_stats_arrived(cnn: Pool<Postgres>, license: &str, node: &str) -> Result<(), StatsHostError> {
    // Does the node exist?
    sqlx::query("UPDATE shaper_nodes SET last_seen=NOW() WHERE license_key=$1 AND node_id=$2")
        .bind(license)
        .bind(node)
        .execute(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    Ok(())
}

#[derive(Clone, sqlx::FromRow, Debug)]
pub struct NodeStatus {
    pub node_id: String,
    pub node_name: String,
    pub last_seen: i32,
}

pub async fn node_status(cnn: &Pool<Postgres>, license: &str) -> Result<Vec<NodeStatus>, StatsHostError> {
    let res = sqlx::query_as::<_, NodeStatus>("SELECT node_id, node_name, extract('epoch' from NOW()-last_seen)::integer AS last_seen FROM shaper_nodes WHERE license_key=$1")
        .bind(license)
        .fetch_all(cnn)
        .await;

    match res {
        Err(e) => {
            log::error!("Unable to get node status: {}", e);
            Err(StatsHostError::DatabaseError(e.to_string()))
        }
        Ok(rows) => Ok(rows)
    }
}