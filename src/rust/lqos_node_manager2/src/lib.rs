use axum::{Router, routing::get};
use tokio::spawn;
use tokio::sync::mpsc::Sender;
use tower_http::services::{ServeDir, ServeFile};
use crate::websocket::ws_handler;
use tracker::*;

mod websocket;
mod tracker;

#[derive(Clone, Debug)]
pub enum ChangeAnnouncement {
    FlowCount(usize),
    ShapedDeviceCount(usize),
}

pub async fn run() -> Sender<ChangeAnnouncement> {
    let (tx,rx) = tokio::sync::mpsc::channel::<ChangeAnnouncement>(100);

    // Announcement handler
    tokio::spawn(tracker::track_changes(rx));
    tokio::spawn(webserver());
    
    tx
}
async fn webserver() {
    // Static file handler
    let serve_dir = ServeDir::new("../lqos_node_manager2/static")
        .not_found_service(ServeFile::new("../lqos_node_manager2/static/index.html"));

    // Create a router
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .nest_service("/", serve_dir);

    // Listen on port 9123 (FIXME: Support IPv6)
    let listener = tokio::net::TcpListener::bind("127.0.0.1:9123")
        .await
        .unwrap();

    log::warn!("Node Manager listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
