mod publish_subscribe;
mod published_channels;
mod ticker;

use std::str::FromStr;
use std::sync::Arc;
use axum::{extract::{ws::{Message, WebSocket}, WebSocketUpgrade}, response::IntoResponse, routing::get, Extension, Router};
use serde::Deserialize;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::node_manager::ws::ticker::channel_ticker;

pub fn websocket_router() -> Router {
    let channels = PubSub::new();
    tokio::spawn(channel_ticker(channels.clone()));
    tokio::spawn(ticker::system_info::cache::update_cache());
    Router::new()
        .route("/ws", get(ws_handler))
        .layer(Extension(channels))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Extension(channels): Extension<Arc<PubSub>>,
) -> impl IntoResponse {
    log::info!("WS Upgrade Called");
    let channels = channels.clone();
    ws.on_upgrade(move |socket| async {
        handle_socket(socket, channels).await;
    })
}

#[derive(Deserialize)]
struct Subscribe {
    channel: String,
}

async fn handle_socket(mut socket: WebSocket, channels: Arc<PubSub>) {
    log::info!("Websocket connected");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(10);
    loop {
        tokio::select! {
            inbound = socket.recv() => {
                // Received a websocket message
                match inbound {
                    Some(Ok(msg)) => {
                        log::info!("Received message: {:?}", msg);
                        if let Ok(text) = msg.to_text() {
                            if let Ok(sub) = serde_json::from_str::<Subscribe>(text) {
                                channels.subscribe(PublishedChannels::from_str(&sub.channel).unwrap(), tx.clone()).await;
                            }
                        }
                    }
                    Some(Err(e)) => {
                        log::warn!("Error receiving websocket message: {:?}", e);
                        break;
                    }
                    None => {
                        break;
                    }
                }
            }
            outbound = rx.recv() => {
                match outbound {
                    Some(msg) => {
                        socket.send(Message::Text(msg)).await.unwrap();
                    }
                    None => {
                        log::info!("WebSocket Disconnected");
                        break;
                    }
                }
            }
        }
    }
    log::info!("Websocket disconnected");
}