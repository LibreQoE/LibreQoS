use axum::{
    extract::{ws::{WebSocket, WebSocketUpgrade, Message}, State},
    response::IntoResponse,
};
use pgdb::sqlx::{Pool, Postgres};
use serde_json::Value;
mod login;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Pool<Postgres>>) -> impl IntoResponse {
    ws.on_upgrade(move |sock| handle_socket(sock, state))
}

async fn handle_socket(mut socket: WebSocket, cnn: Pool<Postgres>) {
    log::info!("WebSocket Connected");
    while let Some(msg) = socket.recv().await {
        let cnn = cnn.clone();
        let msg = msg.unwrap();
        log::info!("Received a message: {:?}", msg);
        if let Ok(text) = msg.into_text() {
            if let Ok(json) = serde_json::from_str::<Value>(&text) {
                log::info!("Received a JSON: {:?}", json);
                if let Some(Value::String(msg_type)) = json.get("msg") {
                    match msg_type.as_str() {
                        "login" => login::on_login(&json, &mut socket, cnn).await,
                        _ => {
                            log::warn!("Unknown message type: {msg_type}");
                        }
                    }
                }
            }
        }
    }
}
