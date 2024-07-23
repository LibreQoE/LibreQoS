use std::path::Path;
use axum::Router;
use log::info;
use tokio::net::TcpListener;
use anyhow::{bail, Result};
use axum::response::Redirect;
use axum::routing::{get, post};
use tower_http::services::ServeDir;
use lqos_config::load_config;
use crate::node_manager::{auth, static_pages::{static_routes, vendor_route}, ws::websocket_router};
use crate::node_manager::local_api::local_api;

/// Launches the Axum webserver to take over node manager duties.
/// This is designed to be run as an independent Tokio future,
/// with tokio::spawn unless you want it to block execution.
pub async fn spawn_webserver() -> Result<()>  {
    // TODO: port change is temporary
    let listener = TcpListener::bind(":::9123").await?;

    // Check that static content is available and setup the path
    let config = load_config()?;
    let static_path = Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2");

    if !static_path.exists() {
        bail!("Static path not found for webserver (vin/static2/");
    }

    // Construct the router from parts
    let router = Router::new()
        .route("/", get(redirect_to_index))
        .route("/doLogin", post(auth::try_login))
        .route("/firstLogin", post(auth::first_user))
        .nest("/websocket/", websocket_router())
        .nest("/vendor", vendor_route()?) // Serve /vendor as purely static
        .nest("/", static_routes()?)
        .nest("/local-api", local_api())
        .fallback_service(ServeDir::new(static_path));

    info!("Webserver listening on :: port 9223");
    axum::serve(listener, router).await?;
    Ok(())
}

/// Provides a redirect service that sends visitors
/// to the index.html page.
async fn redirect_to_index() -> Redirect {
    Redirect::permanent("/index.html")
}