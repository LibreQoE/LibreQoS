use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, ConnectInfo}, response::IntoResponse, TypedHeader, headers,
};
use serde_json::Value;
use std::{net::SocketAddr, path::PathBuf};

pub async fn ws_handler(
	ws: WebSocketUpgrade,
) -> impl IntoResponse {
	ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    log::info!("WebSocket Connected");
    while let Some(msg) = socket.recv().await {
        let msg = msg.unwrap();
        log::info!("Received a message: {:?}", msg);
        if let Ok(text) = msg.into_text() {
            if let Ok(json) = serde_json::from_str::<Value>(&text) {
                log::info!("Received a JSON: {:?}", json);
                if let Some(Value::String(msg_type)) = json.get("msg") {
                    match msg_type.as_str() {
                        "login" => {}
                        _ => {
                            log::warn!("Unknown message type: {msg_type}");
                        }
                    }
                }
            }
        }
    }
}