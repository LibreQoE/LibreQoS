use axum::Router;
use log::info;
use tokio::net::TcpListener;
use anyhow::Result;
use crate::node_manager::{static_pages::{static_routes, vendor_route}, ws::websocket_router};

/// Launches the Axum webserver to take over node manager duties.
/// This is designed to be run as an independent Tokio future,
/// with tokio::spawn unless you want it to block execution.
pub async fn spawn_webserver() -> Result<()>  {
    // TODO: port change is temporary
    let listener = TcpListener::bind(":::9223").await?;

    // Construct the router from parts
    let router = Router::new()
        .nest("/", websocket_router())
        .nest("/vendor", vendor_route()?) // Serve /vendor as purely static
        .nest("/", static_routes()?);

    info!("Webserver listening on :: port 9123");
    axum::serve(listener, router).await?;
    Ok(())
}