use axum::{response::Html, routing::get, Router};
use tokio::spawn;
use tower_http::services::{ServeDir, ServeFile};

pub async fn launch_node_manager() {
    spawn(run());
}

async fn run() {
    // Static file handler
    let serve_dir = ServeDir::new("../lqos_node_manager2/static")
        .not_found_service(ServeFile::new("../lqos_node_manager2/static/index.html"));
    
    // Create a router
    let app = Router::new()
        .nest_service("/", serve_dir);

    // Listen on port 9123 (FIXME: Support IPv6)
    let listener = tokio::net::TcpListener::bind("127.0.0.1:9123")
        .await
        .unwrap();

    log::warn!("Node Manager listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
