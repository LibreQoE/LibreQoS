use axum::extract::ws::WebSocket;
use pgdb::sqlx::{Pool, Postgres};

use crate::web::wss::send_response;

pub async fn send_site_parents(
    cnn: Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    site_name: &str,
) {
    if let Ok(parents) = pgdb::get_parent_list(cnn.clone(), key, site_name).await {
        send_response(socket, wasm_pipe_types::WasmResponse::SiteParents { data: parents }).await;
    }

    let child_result = pgdb::get_child_list(cnn, key, site_name).await;
    if let Ok(children) = child_result {
        send_response(socket, wasm_pipe_types::WasmResponse::SiteChildren { data: children }).await;
    } else {
        log::error!("Error getting children: {:?}", child_result);
    }
}
