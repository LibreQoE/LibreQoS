use axum::{
    extract::{ws::{WebSocket, WebSocketUpgrade}, State},
    response::IntoResponse,
};
use pgdb::sqlx::{Pool, Postgres};
use serde_json::Value;
mod login;
mod nodes;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Pool<Postgres>>) -> impl IntoResponse {
    ws.on_upgrade(move |sock| handle_socket(sock, state))
}

async fn handle_socket(mut socket: WebSocket, cnn: Pool<Postgres>) {
    log::info!("WebSocket Connected");
    let mut credentials: Option<login::LoginResult> = None;
    while let Some(msg) = socket.recv().await {
        let cnn = cnn.clone();
        let msg = msg.unwrap();
        log::info!("Received a message: {:?}", msg);
        if let Ok(text) = msg.into_text() {
            let json = serde_json::from_str::<Value>(&text);
            if json.is_err() {
                log::warn!("Unable to parse JSON: {}", json.err().unwrap());
            } else if let Ok(json) = json {
                log::info!("Received a JSON: {:?}", json);

                if let Some(credentials) = &credentials {
                    let _ = pgdb::refresh_token(cnn.clone(), &credentials.token).await;
                }

                if let Some(Value::String(msg_type)) = json.get("msg") {
                    match msg_type.as_str() {
                        "login" => { // A full login request
                            let result = login::on_login(&json, &mut socket, cnn).await;
                            if let Some(result) = result {
                                credentials = Some(result);
                            }
                        }
                        "auth" => { // Login with just a token
                            let result = login::on_token_auth(&json, &mut socket, cnn).await;
                            if let Some(result) = result {
                                credentials = Some(result);
                            }
                        }
                        "nodeStatus" => {
                            if let Some(credentials) = &credentials {
                                nodes::node_status(cnn.clone(), &mut socket, &credentials.license_key).await;
                            } else {
                                log::info!("Node status requested but no credentials provided");
                            }
                        }
                        _ => {
                            log::warn!("Unknown message type: {msg_type}");
                        }
                    }
                }
            }
        }
    }
}
