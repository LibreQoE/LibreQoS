use sqlx::{FromRow, Pool, Postgres};
use crate::license::StatsHostError;

#[derive(Debug, FromRow)]
pub struct TreeNode {
    pub site_name: String,
    pub index: i32,
    pub parent: i32,
    pub site_type: String,
}

pub async fn get_site_tree(
    cnn: Pool<Postgres>,
    key: &str,
    host_id: &str,
) -> Result<Vec<TreeNode>, StatsHostError> {
    sqlx::query_as::<_, TreeNode>("SELECT site_name, index, parent, site_type FROM site_tree WHERE key = $1 AND host_id=$2")
        .bind(key)
        .bind(host_id)
        .fetch_all(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))
}