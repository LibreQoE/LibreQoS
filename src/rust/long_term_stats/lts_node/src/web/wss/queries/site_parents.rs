use axum::extract::ws::{WebSocket, Message};
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;

pub async fn send_site_parents(
    cnn: Pool<Postgres>,
    socket: &mut WebSocket,
    key: &str,
    site_name: &str,
) {
    if let Ok(parents) = pgdb::get_parent_list(cnn.clone(), key, site_name).await {
        let msg = TreeMessage {
            msg: "site_parents".to_string(),
            data: parents,
        };
        let json = serde_json::to_string(&msg).unwrap();
        if let Err(e) = socket.send(Message::Text(json)).await {
            tracing::error!("Error sending message: {}", e);
        }
    }

    let child_result = pgdb::get_child_list(cnn, key, site_name).await;
    if let Ok(children) = child_result {
        let msg = TreeChildMessage {
            msg: "site_children".to_string(),
            data: children,
        };
        let json = serde_json::to_string(&msg).unwrap();
        if let Err(e) = socket.send(Message::Text(json)).await {
            tracing::error!("Error sending message: {}", e);
        }
    } else {
        log::error!("Error getting children: {:?}", child_result);
    }
}

#[derive(Serialize)]
struct TreeMessage {
    msg: String,
    data: Vec<(String, String)>,
}

#[derive(Serialize)]
struct TreeChildMessage {
    msg: String,
    data: Vec<(String, String, String)>,
}
