mod circuit;
mod ping_monitor;
mod flows_by_circuit;
mod cake_watcher;

use axum::Extension;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{Message, WebSocket};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use tokio::spawn;
use tracing::{debug, info};
use crate::node_manager::ws::single_user_channels::cake_watcher::cake_watcher;
use crate::node_manager::ws::single_user_channels::circuit::circuit_watcher;
use crate::node_manager::ws::single_user_channels::flows_by_circuit::flows_by_circuit;
use crate::node_manager::ws::single_user_channels::ping_monitor::ping_monitor;

#[derive(Serialize, Deserialize)]
enum PrivateChannel {
    CircuitWatcher { circuit: String },
    PingMonitor { ips: Vec<(String, String)> },
    FlowsByCircuit { circuit: String },
    CakeWatcher { circuit: String },
}

pub(super) async fn private_channel_ws_handler(
    ws: WebSocketUpgrade,
    Extension(bus_tx): Extension<tokio::sync::mpsc::Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, lqos_bus::BusRequest)>>,
) -> impl IntoResponse {
    info!("WS Upgrade Called");
    let my_bus = bus_tx.clone();
    ws.on_upgrade(move |socket| async {
        handle_socket(socket, my_bus).await;
    })
}

async fn handle_socket(
    mut socket: WebSocket,
    bus_tx: tokio::sync::mpsc::Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, lqos_bus::BusRequest)>,
) {
    debug!("Websocket connected");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(10);
    loop {
        tokio::select! {
            inbound = socket.recv() => {
                // Handle incoming message - select a private message source
                match inbound {
                    Some(Ok(msg)) => {
                        if let Ok(text) = msg.to_text() {
                            if let Ok(sub) = serde_json::from_str::<PrivateChannel>(text) {
                                match sub {
                                    PrivateChannel::CircuitWatcher {circuit } => {
                                        spawn(circuit_watcher(circuit, tx.clone(), bus_tx.clone()));
                                    },
                                    PrivateChannel::PingMonitor { ips } => {
                                        spawn(ping_monitor(ips, tx.clone()));
                                    },
                                    PrivateChannel::FlowsByCircuit { circuit } => {
                                        spawn(flows_by_circuit(circuit, tx.clone()));
                                    },
                                    PrivateChannel::CakeWatcher { circuit } => {
                                        spawn(cake_watcher(circuit, tx.clone()));
                                    },
                                }
                            } else {
                                debug!("Failed to parse private message: {:?}", text);
                            }
                        }
                    }
                    Some(Err(_)) => break,
                    None => break,
                }
            }
            outbound = rx.recv() => {
                match outbound {
                    Some(msg) => {
                        if let Err(_) = socket.send(Message::Text(msg)).await {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
}
