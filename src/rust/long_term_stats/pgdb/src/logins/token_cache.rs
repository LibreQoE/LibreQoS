use super::LoginDetails;
use crate::license::StatsHostError;
use dashmap::DashMap;
use lqos_utils::unix_time::unix_now;
use once_cell::sync::Lazy;
use sqlx::{Pool, Postgres, Row};

static TOKEN_CACHE: Lazy<DashMap<String, TokenDetails>> = Lazy::new(DashMap::new);

struct TokenDetails {
    last_seen: u64,
    last_refreshed: u64,
}

pub async fn create_token(
    cnn: &Pool<Postgres>,
    details: &LoginDetails,
    key: &str,
    username: &str,
) -> Result<(), StatsHostError> {
    sqlx::query("INSERT INTO active_tokens (token, key, username) VALUES ($1, $2, $3)")
        .bind(&details.token)
        .bind(key)
        .bind(username)
        .execute(cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let now = unix_now().unwrap_or(0);
    TOKEN_CACHE.insert(
        details.token.clone(),
        TokenDetails {
            last_seen: now,
            last_refreshed: now,
        },
    );

    Ok(())
}

pub async fn refresh_token(cnn: Pool<Postgres>, token_id: &str) -> Result<(), StatsHostError> {
    if let Some(mut token) = TOKEN_CACHE.get_mut(token_id) {
        let now = unix_now().unwrap_or(0);
        token.last_seen = now;
        let age = now - token.last_refreshed;

        if age > 300 {
            token.last_refreshed = now;
            sqlx::query("UPDATE active_tokens SET last_seen = NOW() WHERE token = $1")
                .bind(token_id)
                .execute(&cnn)
                .await
                .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
        }

        Ok(())
    } else {
        Err(StatsHostError::DatabaseError("Unauthorized".to_string()))
    }
}

pub async fn token_to_credentials(
    cnn: Pool<Postgres>,
    token_id: &str,
) -> Result<LoginDetails, StatsHostError> {
    let row = sqlx::query("SELECT key, username FROM active_tokens WHERE token = $1")
        .bind(token_id)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let key: String = row
        .try_get("key")
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    let username: String = row
        .try_get("username")
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let row = sqlx::query("SELECT nicename FROM logins WHERE key = $1 AND username = $2")
        .bind(&key)
        .bind(username)
        .fetch_one(&cnn)
        .await
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;

    let nicename: String = row
        .try_get("nicename")
        .map_err(|e| StatsHostError::DatabaseError(e.to_string()))?;
    let details = LoginDetails {
        token: token_id.to_string(),
        name: nicename,
        license: key,
    };

    Ok(details)
}
