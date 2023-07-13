use axum::extract::ws::WebSocket;
use pgdb::sqlx::{Pool, Postgres};
use wasm_pipe_types::Node;

use crate::web::wss::send_response;

fn convert(ns: pgdb::NodeStatus) -> Node {
    Node {
        node_id: ns.node_id,
        node_name: ns.node_name,
        last_seen: ns.last_seen,
    }
}

pub async fn node_status(cnn: &Pool<Postgres>, socket: &mut WebSocket, key: &str) {
    tracing::info!("Fetching node status, {key}");
    let nodes = pgdb::node_status(cnn, key).await;
    match nodes {
        Ok(nodes) => {
            let nodes: Vec<Node> = nodes.into_iter().map(convert).collect();
            send_response(socket, wasm_pipe_types::WasmResponse::NodeStatus { nodes }).await;
        },
        Err(e) => {
            tracing::error!("Unable to obtain node status: {}", e);
        }
    }
}