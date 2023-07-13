use axum::extract::ws::WebSocket;
use pgdb::sqlx::{Pool, Postgres};

use crate::web::wss::send_response;

pub async fn send_site_parents(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    site_name: &str,
) {
    if let Ok(parents) = pgdb::get_parent_list(cnn, key, site_name).await {
        send_response(socket, wasm_pipe_types::WasmResponse::SiteParents { data: parents }).await;
    }

    let child_result = pgdb::get_child_list(cnn, key, site_name).await;
    if let Ok(children) = child_result {
        send_response(socket, wasm_pipe_types::WasmResponse::SiteChildren { data: children }).await;
    } else {
        tracing::error!("Error getting children: {:?}", child_result);
    }
}

pub async fn send_circuit_parents(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    circuit_id: &str,
) {
    if let Ok(parents) = pgdb::get_circuit_parent_list(cnn, key, circuit_id).await {
        send_response(socket, wasm_pipe_types::WasmResponse::SiteParents { data: parents }).await;
    }
}

pub async fn send_root_parents(
    cnn: &Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
) {
    let site_name = "Root";
    let child_result = pgdb::get_child_list(cnn, key, site_name).await;
    if let Ok(children) = child_result {
        send_response(socket, wasm_pipe_types::WasmResponse::SiteChildren { data: children }).await;
    } else {
        tracing::error!("Error getting children: {:?}", child_result);
    }
}