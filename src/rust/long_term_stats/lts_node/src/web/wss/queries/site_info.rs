use axum::extract::ws::{WebSocket, Message};
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;
use super::site_tree::SiteTree;

#[derive(Serialize)]
struct SiteInfoMessage {
    msg: String,
    data: SiteTree,
}


pub async fn send_site_info(cnn: Pool<Postgres>, socket: &mut WebSocket, key: &str, site_id: &str) {
    if let Ok(host) = pgdb::get_site_info(cnn, key, site_id).await {
        let host = SiteTree::from(host);
        let msg = SiteInfoMessage {
            msg: "site_info".to_string(),
            data: host,
        };
        let json = serde_json::to_string(&msg).unwrap();
        if let Err(e) = socket.send(Message::Text(json)).await {
            tracing::error!("Error sending message: {}", e);
        }
    }
}