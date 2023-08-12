use tracing_subscriber::fmt::format::FmtSpan;
mod pki;
mod server;

fn set_console_logging() -> anyhow::Result<()> {
    // install global collector configured based on RUST_LOG env var.
    let subscriber = tracing_subscriber::fmt()
        // Use a more compact, abbreviated log format
        .compact()
        // Display source code file paths
        .with_file(true)
        // Display source code line numbers
        .with_line_number(true)
        // Display the thread ID an event was recorded on
        .with_thread_ids(true)
        // Don't display the event's target (module path)
        .with_target(false)
        // Include per-span timings
        .with_span_events(FmtSpan::CLOSE)
        // Build the subscriber
        .finish();

    // Set the subscriber as the default
    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Obtain the database pool
    let pool = pgdb::get_connection_pool(10).await;
    if pool.is_err() {
        tracing::error!("Unable to connect to the database");
        tracing::error!("{pool:?}");
        return Err(anyhow::Error::msg("Unable to connect to the database"));
    }
    let pool = pool.unwrap();

    // Start the logger
    set_console_logging().unwrap();

    let _ = server::listen_accept(pool.clone()).await;
    Ok(())
}
