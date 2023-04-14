mod submissions;
mod web;
mod pki;


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Start the logger
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "warn"),
    );

    // Start the webserver
    {
        log::info!("Starting the webserver");
        tokio::spawn(web::webserver());
    }

    // Start the submissions serer
    log::info!("Starting the submissions server");
    if let Err(e) = tokio::spawn(submissions::submissions_server()).await {
        log::error!("Server exited with error: {}", e);
    }

    Ok(())
}
