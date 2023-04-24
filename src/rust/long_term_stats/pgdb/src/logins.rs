use sha2::Sha256;
use sha2::Digest;
use sqlx::{Pool, Postgres, Row};
use uuid::Uuid;
use crate::license::StatsHostError;

#[derive(Debug)]
pub struct LoginDetails {
    pub token: String,
    pub license: String,
    pub name: String,
}

fn hash_password(password: &str) -> String {
    let salted = format!("!x{password}_SaltIsGoodForYou");
    let mut sha256 = Sha256::new();
    sha256.update(salted);
    format!("{:X}", sha256.finalize())
  }

pub async fn try_login(cnn: Pool<Postgres>, key: &str, username: &str, password: &str) -> Result<LoginDetails, StatsHostError> {
    let password = hash_password(password);

    let row = sqlx::query("SELECT nicename FROM logins WHERE key = $1 AND username = $2 AND password_hash = $3")
        .bind(key)
        .bind(username)
        .bind(password)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let nicename: String = row.try_get("nicename").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    let uuid = Uuid::new_v4().to_string();
    let details = LoginDetails {
        token: uuid,
        name: nicename,
        license: key.to_string(),
    };

    sqlx::query("INSERT INTO active_tokens (token, key, username) VALUES ($1, $2, $3)")
        .bind(&details.token)
        .bind(key)
        .bind(username)
        .execute(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    Ok(details)
}

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

pub async fn refresh_token(cnn: Pool<Postgres>, token_id: &str) -> Result<(), StatsHostError> {
    sqlx::query("UPDATE active_tokens SET last_seen = NOW() WHERE token = $1")
        .bind(token_id)
        .execute(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    Ok(())
}

pub async fn token_to_credentials(cnn: Pool<Postgres>, token_id: &str) -> Result<LoginDetails, StatsHostError> {
    let row = sqlx::query("SELECT key, username FROM active_tokens WHERE token = $1")
        .bind(token_id)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let key: String = row.try_get("key").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    let username: String = row.try_get("username").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let row = sqlx::query("SELECT nicename FROM logins WHERE key = $1 AND username = $2")
        .bind(&key)
        .bind(username)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let nicename: String = row.try_get("nicename").map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    let details = LoginDetails {
        token: token_id.to_string(),
        name: nicename,
        license: key,
    };

    Ok(details)
}