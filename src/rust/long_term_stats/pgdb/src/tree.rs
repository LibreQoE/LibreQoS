use sqlx::{FromRow, Pool, Postgres, Row};
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
    cnn: &Pool<Postgres>,
    key: &str,
    host_id: &str,
) -> Result<Vec<TreeNode>, StatsHostError> {
    sqlx::query_as::<_, TreeNode>("SELECT site_name, index, parent, site_type, max_down, max_up, current_down, current_up, current_rtt FROM site_tree WHERE key = $1 AND host_id=$2")
        .bind(key)
        .bind(host_id)
        .fetch_all(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))
}

pub async fn get_site_info(
    cnn: &Pool<Postgres>,
    key: &str,
    site_name: &str,
) -> Result<TreeNode, StatsHostError> {
    sqlx::query_as::<_, TreeNode>("SELECT site_name, index, parent, site_type, max_down, max_up, current_down, current_up, current_rtt FROM site_tree WHERE key = $1 AND site_name=$2")
        .bind(key)
        .bind(site_name)
        .fetch_one(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))
}

pub async fn get_site_id_from_name(
    cnn: &Pool<Postgres>,
    key: &str,
    site_name: &str,
) -> Result<i32, StatsHostError> {
    if site_name == "root" {
        return Ok(0);
    }
    let site_id_db = sqlx::query("SELECT index FROM site_tree WHERE key = $1 AND site_name=$2")
        .bind(key)
        .bind(site_name)
        .fetch_one(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    let site_id: i32 = site_id_db.try_get("index").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    Ok(site_id)
}

pub async fn get_parent_list(
    cnn: &Pool<Postgres>,
    key: &str,
    site_name: &str,
) -> Result<Vec<(String, String)>, StatsHostError> {
    let mut result = Vec::new();

    // Get the site index
    let site_id_db = sqlx::query("SELECT index FROM site_tree WHERE key = $1 AND site_name=$2")
        .bind(key)
        .bind(site_name)
        .fetch_one(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    let mut site_id: i32 = site_id_db.try_get("index").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    // Get the parent list
    while site_id != 0 {
        let parent_db = sqlx::query("SELECT site_name, parent, site_type FROM site_tree WHERE key = $1 AND index=$2")
            .bind(key)
            .bind(site_id)
            .fetch_one(cnn)
            .await
            .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        let parent: String = parent_db.try_get("site_name").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        let site_type: String = parent_db.try_get("site_type").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        site_id = parent_db.try_get("parent").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        result.push((site_type, parent));
    }

    Ok(result)
}

pub async fn get_child_list(
    cnn: &Pool<Postgres>,
    key: &str,
    site_name: &str,
) -> Result<Vec<(String, String, String)>, StatsHostError> {
    let mut result = Vec::new();

    // Get the site index
    let site_id_db = sqlx::query("SELECT index FROM site_tree WHERE key = $1 AND site_name=$2")
        .bind(key)
        .bind(site_name)
        .fetch_one(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    let site_id: i32 = site_id_db.try_get("index").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    // Add child sites
    let child_sites = sqlx::query("SELECT site_name, parent, site_type FROM site_tree WHERE key=$1 AND parent=$2")
        .bind(key)
        .bind(site_id)
        .fetch_all(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    for child in child_sites {
        let child_name: String = child.try_get("site_name").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        let child_type: String = child.try_get("site_type").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        result.push((child_type, child_name.clone(), child_name));
    }

    // Add child shaper nodes
    let child_circuits = sqlx::query("SELECT circuit_id, circuit_name FROM shaped_devices WHERE key=$1 AND parent_node=$2")
        .bind(key)
        .bind(site_name)
        .fetch_all(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    for child in child_circuits {
        let child_name: String = child.try_get("circuit_name").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        let child_id: String = child.try_get("circuit_id").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        result.push(("circuit".to_string(), child_id, child_name));
    }

    result.sort_by(|a, b| a.2.cmp(&b.2));

    Ok(result)
}

pub async fn get_circuit_parent_list(
    cnn: &Pool<Postgres>,
    key: &str,
    circuit_id: &str,
) -> Result<Vec<(String, String)>, StatsHostError> {
    let mut result = Vec::new();

    // Get the site name to start at
    let site_name : String = sqlx::query("SELECT parent_node FROM shaped_devices WHERE key = $1 AND circuit_id= $2")
        .bind(key)
        .bind(circuit_id)
        .fetch_one(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?
        .get(0);

    // Get the site index
    let site_id_db = sqlx::query("SELECT index FROM site_tree WHERE key = $1 AND site_name=$2")
        .bind(key)
        .bind(site_name)
        .fetch_one(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    let mut site_id: i32 = site_id_db.try_get("index").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    // Get the parent list
    while site_id != 0 {
        let parent_db = sqlx::query("SELECT site_name, parent, site_type FROM site_tree WHERE key = $1 AND index=$2")
            .bind(key)
            .bind(site_id)
            .fetch_one(cnn)
            .await
            .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        let parent: String = parent_db.try_get("site_name").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        let site_type: String = parent_db.try_get("site_type").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        site_id = parent_db.try_get("parent").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        result.push((site_type, parent));
    }

    Ok(result)
}