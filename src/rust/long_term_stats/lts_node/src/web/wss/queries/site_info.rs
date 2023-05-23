use axum::extract::ws::WebSocket;
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;
use wasm_pipe_types::{SiteTree, WasmResponse};
use crate::web::wss::send_response;
use super::site_tree::tree_to_host;

#[derive(Serialize)]
struct SiteInfoMessage {
    msg: String,
    data: SiteTree,
}


pub async fn send_site_info(cnn: &Pool<Postgres>, socket: &mut WebSocket, key: &str, site_id: &str) {
    if let Ok(host) = pgdb::get_site_info(cnn, key, site_id).await {
        let host = tree_to_host(host);
        send_response(socket, WasmResponse::SiteInfo { data: host }).await;
    }
}