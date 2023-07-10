use sqlx::{Pool, Postgres};

use crate::license::StatsHostError;

use super::hasher::hash_password;

pub async fn delete_user(cnn: Pool<Postgres>, key: &str, username: &str) -> Result<(), StatsHostError> {
    sqlx::query("DELETE FROM logins WHERE key = $1 AND username = $2")
        .bind(key)
        .bind(username)
        .execute(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    Ok(())
}

pub async fn add_user(cnn: Pool<Postgres>, key: &str, username: &str, password: &str, nicename: &str) -> Result<(), StatsHostError> {
    let password = hash_password(password);
    sqlx::query("INSERT INTO logins (key, username, password_hash, nicename) VALUES ($1, $2, $3, $4)")
        .bind(key)
        .bind(username)
        .bind(password)
        .bind(nicename)
        .execute(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    Ok(())
}