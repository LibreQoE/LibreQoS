use sqlx::{FromRow, Pool, Postgres};
use crate::license::StatsHostError;

#[derive(Debug, FromRow)]
pub struct TreeNode {
    pub site_name: String,
    pub index: i32,
    pub parent: i32,
    pub site_type: String,
    pub max_down: i32,
    pub max_up: i32,
    pub current_down: i32,
    pub current_up: i32,
    pub current_rtt: i32,
}

pub async fn get_site_tree(
    cnn: Pool<Postgres>,
    key: &str,
    host_id: &str,
) -> Result<Vec<TreeNode>, StatsHostError> {
    sqlx::query_as::<_, TreeNode>("SELECT site_name, index, parent, site_type, max_down, max_up, current_down, current_up, current_rtt FROM site_tree WHERE key = $1 AND host_id=$2")
        .bind(key)
        .bind(host_id)
        .fetch_all(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))
}

pub async fn get_site_info(
    cnn: Pool<Postgres>,
    key: &str,
    site_name: &str,
) -> Result<TreeNode, StatsHostError> {
    sqlx::query_as::<_, TreeNode>("SELECT site_name, index, parent, site_type, max_down, max_up, current_down, current_up, current_rtt FROM site_tree WHERE key = $1 AND site_name=$2")
        .bind(key)
        .bind(site_name)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))
}
