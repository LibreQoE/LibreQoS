use crate::lts2_sys::control_channel::ControlChannelCommand;
use crate::node_manager::local_api::local_api;
use crate::node_manager::shaper_queries_actor::shaper_queries_actor;
use crate::node_manager::{
    auth,
    static_pages::{static_routes, vendor_route},
    ws::websocket_router,
};
use crate::system_stats::SystemStats;
use anyhow::{Result, bail};
use axum::Router;
use axum::http::StatusCode;
use axum::response::Redirect;
use axum::routing::{get, post};
use lqos_bus::BusRequest;
use lqos_config::load_config;
use std::path::Path;
use tokio::net::TcpListener;
use tokio::sync::mpsc::Sender;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing::info;

/// Launches the Axum webserver to take over node manager duties.
/// This is designed to be run as an independent Tokio future,
/// with tokio::spawn unless you want it to block execution.
pub async fn spawn_webserver(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
    control_tx: tokio::sync::mpsc::Sender<ControlChannelCommand>,
) -> Result<()> {
    // Check that static content is available and set up the path
    let config = load_config()?;
    let static_path = Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2");

    if !static_path.exists() {
        bail!("Static path not found for webserver (bin/static2/");
    }

    // Listen for net connections
    let listen_address = config
        .webserver_listen
        .clone()
        .unwrap_or(":::9123".to_string());
    let listener = TcpListener::bind(&listen_address).await?;

    // Setup shaper queries
    let shaper_tx = shaper_queries_actor(control_tx.clone()).await;

    // Construct the router from parts
    let router = Router::new()
        .route("/", get(redirect_to_index))
        .route("/doLogin", post(auth::try_login))
        .route("/firstLogin", post(auth::first_user))
        .route("/health", get(health_check))
        // Backwards compatible aliases for historical misspellings.
        .route_service(
            "/config_spylnx.js",
            ServeFile::new(static_path.join("config_splynx.js")),
        )
        .route_service(
            "/config_spylnx.js.map",
            ServeFile::new(static_path.join("config_splynx.js.map")),
        )
        .nest(
            "/websocket/",
            websocket_router(
                bus_tx.clone(),
                system_usage_tx.clone(),
                control_tx.clone(),
                shaper_tx.clone(),
            ),
        )
        .nest("/vendor", vendor_route()?) // Serve /vendor as purely static
        .nest("/", static_routes()?)
        .nest("/local-api", local_api(shaper_tx))
        .fallback_service(ServeDir::new(static_path))
        .layer(CorsLayer::very_permissive());

    info!("Webserver listening on: [{listen_address}]");
    axum::serve(listener, router).await?;
    Ok(())
}

/// Provides a redirect service that sends visitors
/// to the index.html page.
async fn redirect_to_index() -> Redirect {
    Redirect::permanent("/index.html")
}

/// Provides a simple OK status
async fn health_check() -> (StatusCode, &'static str) {
    (StatusCode::OK, "OK")
}
