//! Websocket handling for the node manager. This module provides a websocket router that can be mounted in the main application.
//! There are two major types of websocket connection supported:
//! * General websocket connections that allow for subscribing to multiple channels, using the `/ws` route.
//! * Private websocket connections that allow for subscribing to a single channel, using the `/private_ws` route.
//!
//! General websocket connections are multi-user, and based on a time-based "ticker". They send out updates
//! to all subscribers at a regular interval, sharing the latest information about the system.
//!
//! Private websocket connections are single-user, and are used for more specific information. They are used
//! for things like monitoring a specific user, or for receiving updates about a specific user.
//!
//! Both types of websocket are authenticated using the auth layer.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use axum::{Extension, extract::{WebSocketUpgrade, ws::{Message, WebSocket}}, response::IntoResponse, Router, routing::get};
use serde::Deserialize;
use tokio::sync::mpsc::Sender;
use tracing::debug;
use crate::node_manager::auth::auth_layer;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::node_manager::ws::ticker::channel_ticker;

mod publish_subscribe;
mod published_channels;
mod ticker;
mod single_user_channels;

/// Provides an Axum router for the websocket system. Exposes two routes:
/// * /ws: A general websocket route that allows for subscribing to multiple channels
/// * /private_ws: A private websocket route that allows for subscribing to a single channel
///
/// Returns a router that can be mounted in the main application.
pub fn websocket_router() -> Router {
    let channels = PubSub::new();
    tokio::spawn(channel_ticker(channels.clone()));
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
    debug!("WS Upgrade Called");
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
    debug!("Websocket connected");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Arc<String>>(128);
    let mut subscribed_channels = HashSet::new();
    loop {
        tokio::select! {
            inbound = socket.recv() => {
                // Received a websocket message
                match inbound {
                    Some(Ok(msg)) => receive_channel_message(msg, channels.clone(), tx.clone(), &mut subscribed_channels).await,
                    Some(Err(_)) => break, // The channel has closed
                    None => break, // The channel has closed
                }
            }
            outbound = rx.recv() => {
                match outbound {
                    Some(msg) => {
                        if let Err(_) = socket.send(Message::Text((*msg).clone())).await {
                            // The outbound websocket has closed. That's ok, it's not
                            // an error. We're relying on *this* task terminating to in
                            // turn close the subscription channel, which will in turn
                            // cause the subscription to end.
                            break;
                        }
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }
    debug!("Websocket disconnected");
}

async fn receive_channel_message(msg: Message, channels: Arc<PubSub>, tx: Sender<Arc<String>>, subscribed_channels: &mut HashSet<PublishedChannels>) {
    if let Ok(text) = msg.to_text() {
        if let Ok(sub) = serde_json::from_str::<Subscribe>(text) {
            if let Ok(channel) = PublishedChannels::from_str(&sub.channel) {
                if !subscribed_channels.contains(&channel) {
                    channels.subscribe(channel, tx.clone()).await;
                    subscribed_channels.insert(channel);
                }
            }
        }
    }
}
