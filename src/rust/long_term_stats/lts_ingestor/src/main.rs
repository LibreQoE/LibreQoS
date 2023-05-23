use tracing::{error, info};
mod submissions;
mod pki;

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

    // Start the submission queue
    let submission_sender = {
        info!("Starting the submission queue");
        submissions::submissions_queue(pool.clone()).await?
    };

    
    // Start the submissions serer
    info!("Starting the submissions server");
    if let Err(e) = tokio::spawn(submissions::submissions_server(pool.clone(), submission_sender)).await {
        error!("Server exited with error: {}", e);
    }
    
    Ok(())
}