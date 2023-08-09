use pgdb::sqlx::{Pool, Postgres};
use tokio::sync::mpsc::Sender;
use tracing::instrument;
use wasm_pipe_types::{Node, WasmResponse};

fn convert(ns: pgdb::NodeStatus) -> Node {
    Node {
        node_id: ns.node_id,
        node_name: ns.node_name,
        last_seen: ns.last_seen,
    }
}

#[instrument(skip(cnn, tx, key))]
pub async fn node_status(cnn: &Pool<Postgres>, tx: Sender<WasmResponse>, key: &str) {
    tracing::info!("Fetching node status, {key}");
    let nodes = pgdb::node_status(cnn, key).await;
    match nodes {
        Ok(nodes) => {
            let nodes: Vec<Node> = nodes.into_iter().map(convert).collect();
            tx.send(wasm_pipe_types::WasmResponse::NodeStatus { nodes }).await.unwrap();
        },
        Err(e) => {
            tracing::error!("Unable to obtain node status: {}", e);
        }
    }
}