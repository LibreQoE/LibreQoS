use axum::extract::ws::{WebSocket, Message};
use pgdb::{
    sqlx::{Pool, Postgres},
    TreeNode,
};
use serde::Serialize;

pub async fn send_site_tree(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str, parent: &str) {
    let tree = pgdb::get_site_tree(cnn, key, parent).await.unwrap();
    let tree = tree
        .into_iter()
        .map(|row| row.into())
        .collect::<Vec<SiteTree>>();
    let msg = TreeMessage {
        msg: "site_tree".to_string(),
        data: tree,
    };
    let json = serde_json::to_string(&msg).unwrap();
    if let Err(e) = socket.send(Message::Text(json)).await {
        tracing::error!("Error sending message: {}", e);
    }
}

#[derive(Serialize)]
pub struct SiteTree {
    pub index: i32,
    pub site_name: String,
    pub site_type: String,
    pub parent: i32,
    pub max_down: i32,
    pub max_up: i32,
    pub current_down: i32,
    pub current_up: i32,
    pub current_rtt: i32,
}

impl From<TreeNode> for SiteTree {
    fn from(row: TreeNode) -> Self {
        SiteTree {
            index: row.index,
            site_name: row.site_name,
            site_type: row.site_type,
            parent: row.parent,
            max_down: row.max_down,
            max_up: row.max_up,
            current_down: row.current_down,
            current_up: row.current_up,
            current_rtt: row.current_rtt,
        }
    }
}

#[derive(Serialize)]
struct TreeMessage {
    msg: String,
    data: Vec<SiteTree>,
}
