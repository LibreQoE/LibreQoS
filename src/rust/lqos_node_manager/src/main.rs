mod node_manager;
mod shaped_devices_tracker;

use tracing::error;
use tracing::level_filters::LevelFilter;
use lqos_config::load_config;
use crate::shaped_devices_tracker::shaped_devices_watcher;

pub fn set_console_logging() -> anyhow::Result<()> {
    // install global collector configured based on RUST_LOG env var.
    let level = if let Ok(level) = std::env::var("RUST_LOG") {
        match level.to_lowercase().as_str() {
            "trace" => LevelFilter::TRACE,
            "debug" => LevelFilter::DEBUG,
            "info" => LevelFilter::INFO,
            "warn" => LevelFilter::WARN,
            "error" => LevelFilter::ERROR,
            _ => LevelFilter::WARN,
        }
    } else {
        LevelFilter::WARN
    };

    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        // Use a more compact, abbreviated log format
        .compact()
        // Display source code file paths
        .with_file(true)
        // Display source code line numbers
        .with_line_number(true)
        // Display the thread ID an event was recorded on
        .with_thread_ids(false)
        // Don't display the event's target (module path)
        .with_target(false)
        // Build the subscriber
        .finish();

    // Set the subscriber as the default
    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    set_console_logging()?;

    let config = load_config()?;
    if config.disable_webserver.unwrap_or(false) {
        error!("Webserver disabled by configuration");
        return Ok(());
    }

    shaped_devices_watcher()?;
    node_manager::spawn_webserver().await?;

    Ok(())
}
