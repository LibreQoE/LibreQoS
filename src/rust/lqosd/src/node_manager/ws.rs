use std::str::FromStr;
use std::sync::Arc;

use axum::{Extension, extract::{WebSocketUpgrade, ws::{Message, WebSocket}}, response::IntoResponse, Router, routing::get};
use serde::Deserialize;
use tokio::sync::mpsc::Sender;

use crate::node_manager::auth::auth_layer;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::node_manager::ws::ticker::channel_ticker;

mod publish_subscribe;
mod published_channels;
mod ticker;
mod single_user_channels;

pub fn websocket_router() -> Router {
    let channels = PubSub::new();
    tokio::spawn(channel_ticker(channels.clone()));
    tokio::spawn(ticker::system_info::cache::update_cache());
    Router::new()
        .route("/private_ws", get(single_user_channels::private_channel_ws_handler))
        .route("/ws", get(ws_handler))
        .route_layer(axum::middleware::from_fn(auth_layer))
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
                    Some(Ok(msg)) => receive_channel_message(msg, channels.clone(), tx.clone()).await,
                    Some(Err(_)) => break, // The channel has closed
                    None => break, // The channel has closed
                }
            }
            outbound = rx.recv() => {
                match outbound {
                    Some(msg) => {
                        if let Err(_) = socket.send(Message::Text(msg)).await {
                            // The outbound websocket has closed. That's ok, it's not
                            // an error. We're relying on *this* task terminating to in
                            // turn close the subscription channel, which will in turn
                            // cause the subscription to end.
                            break;
                        }
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

async fn receive_channel_message(msg: Message, channels: Arc<PubSub>, tx: Sender<String>) {
    log::debug!("Received message: {:?}", msg);
    if let Ok(text) = msg.to_text() {
        if let Ok(sub) = serde_json::from_str::<Subscribe>(text) {
            if let Ok(channel) = PublishedChannels::from_str(&sub.channel) {
                channels.subscribe(channel, tx.clone()).await;
            }
        }
    }
}
