use sqlx::{Pool, Postgres, Row};
use uuid::Uuid;
use crate::license::StatsHostError;
use super::{hasher::hash_password, token_cache::create_token};

#[derive(Debug, Clone)]
pub struct LoginDetails {
    pub token: String,
    pub license: String,
    pub name: String,
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

    create_token(&cnn, &details, key, username).await?;

    Ok(details)
}