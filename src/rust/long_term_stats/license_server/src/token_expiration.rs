use std::time::Duration;
use pgdb::sqlx::{Postgres, Pool};

pub async fn token_expiration_loop(pool: Pool<Postgres>) {
    loop {
        tracing::info!("Checking token expiration");
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        interval.tick().await;
        let result = check_token_expiration(&pool).await;
        if let Err(e) = result {
            tracing::error!("Error checking token expiration: {:?}", e);
        }
    }
}

#[tracing::instrument(skip(pool))]
async fn check_token_expiration(pool: &Pool<Postgres>) -> anyhow::Result<()> {
    pgdb::expire_tokens(pool).await?;
    Ok(())
}