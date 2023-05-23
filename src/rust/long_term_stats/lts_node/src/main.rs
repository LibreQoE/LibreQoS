mod web;
use tracing::{info, error};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    // Get the database connection pool
    let pool = pgdb::get_connection_pool(5).await;
    if pool.is_err() {
        error!("Unable to connect to the database");
        error!("{pool:?}");
        return Err(anyhow::Error::msg("Unable to connect to the database"));
    }
    let pool = pool.unwrap();

    // Start the webserver
    info!("Starting the webserver");
    let _ = tokio::spawn(web::webserver(pool)).await;

    Ok(())
}
