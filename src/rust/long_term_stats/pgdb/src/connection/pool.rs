use sqlx::{postgres::PgPoolOptions, Postgres, Pool};
use super::connection_string::CONNECTION_STRING;

/// Obtain a connection pool to the database.
/// 
/// # Arguments
/// * `max_connections` - The maximum number of connections to the database.
pub async fn get_connection_pool(max_connections: u32) -> Result<Pool<Postgres>, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(&CONNECTION_STRING)
        .await
}
