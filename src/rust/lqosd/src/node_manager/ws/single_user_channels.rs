mod circuit;
mod ping_monitor;

use std::sync::Arc;
use std::time::Duration;
use axum::Extension;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{Message, WebSocket};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use tokio::spawn;
use tokio::time::MissedTickBehavior;
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::single_user_channels::circuit::circuit_watcher;
use crate::node_manager::ws::single_user_channels::ping_monitor::ping_monitor;

#[derive(Serialize, Deserialize)]
enum PrivateChannel {
    CircuitWatcher { circuit: String },
    PingMonitor { ips: Vec<String> },
}

pub(super) async fn private_channel_ws_handler(
    ws: WebSocketUpgrade,
    Extension(channels): Extension<Arc<PubSub>>,
) -> impl IntoResponse {
    log::info!("WS Upgrade Called");
    let channels = channels.clone();
    ws.on_upgrade(move |socket| async {
        handle_socket(socket, channels).await;
    })
}

async fn handle_socket(mut socket: WebSocket, channels: Arc<PubSub>) {
    log::info!("Websocket connected");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(10);
    loop {
        tokio::select! {
            inbound = socket.recv() => {
                // Handle incoming message - select a private message source
                match inbound {
                    Some(Ok(msg)) => {
                        log::info!("Received private message: {:?}", msg);
                        if let Ok(text) = msg.to_text() {
                            if let Ok(sub) = serde_json::from_str::<PrivateChannel>(text) {
                                match sub {
                                    PrivateChannel::CircuitWatcher {circuit } => {
                                        spawn(circuit_watcher(circuit, tx.clone()));
                                    },
                                    PrivateChannel::PingMonitor { ips } => {
                                        spawn(ping_monitor(ips, tx.clone()));
                                    },
                                }
                            } else {
                                log::warn!("Failed to parse private message: {:?}", text);
                                let test = PrivateChannel::CircuitWatcher { circuit: "test".to_string() };
                                let test = serde_json::to_string(&test).unwrap();
                                println!("{test}");
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
}
