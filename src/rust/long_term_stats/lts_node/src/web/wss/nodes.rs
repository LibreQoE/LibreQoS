use axum::extract::ws::WebSocket;
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;

#[derive(Serialize)]
struct NodeStatus {
    msg: String,
    nodes: Vec<Node>,
}

#[derive(Serialize)]
struct Node {
    node_id: String,
    node_name: String,
    last_seen: i32,
}

impl From<pgdb::NodeStatus> for Node {
    fn from(ns: pgdb::NodeStatus) -> Self {
        Self {
            node_id: ns.node_id,
            node_name: ns.node_name,
            last_seen: ns.last_seen,
        }
    }
}

pub async fn node_status(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str) {
    log::info!("Fetching node status, {key}");
    let nodes = pgdb::node_status(cnn, key).await;
    match nodes {
        Ok(nodes) => {
            let nodes: Vec<Node> = nodes.into_iter().map(|n| n.into()).collect();
            let status = NodeStatus {
                msg: "nodeStatus".to_string(),
                nodes};
            let reply = serde_json::to_string(&status).unwrap();
            let msg = axum::extract::ws::Message::Text(reply);
            socket.send(msg).await.unwrap();
        },
        Err(e) => {
            log::error!("Unable to obtain node status: {}", e);
        }
    }
}