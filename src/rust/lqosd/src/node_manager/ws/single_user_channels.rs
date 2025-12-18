mod cake_watcher;
mod chatbot;
mod circuit;
mod flows_by_circuit;
mod ping_monitor;

use crate::node_manager::ws::single_user_channels::cake_watcher::cake_watcher;
use crate::node_manager::ws::single_user_channels::circuit::circuit_watcher;
use crate::node_manager::ws::single_user_channels::flows_by_circuit::flows_by_circuit;
use crate::node_manager::ws::single_user_channels::ping_monitor::ping_monitor;
use axum::Extension;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{Message, WebSocket};
use axum::http::{HeaderMap, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use tokio::spawn;
use tracing::{debug, info};

#[derive(Serialize, Deserialize)]
enum PrivateChannel {
    CircuitWatcher { circuit: String },
    PingMonitor { ips: Vec<(String, String)> },
    FlowsByCircuit { circuit: String },
    CakeWatcher { circuit: String },
    Chatbot { browser_ts_ms: Option<i64> },
}

pub(super) async fn private_channel_ws_handler(
    ws: WebSocketUpgrade,
    Extension(bus_tx): Extension<
        tokio::sync::mpsc::Sender<(
            tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
            lqos_bus::BusRequest,
        )>,
    >,
    Extension(control_tx): Extension<
        tokio::sync::mpsc::Sender<crate::lts2_sys::control_channel::ControlChannelCommand>,
    >,
    headers: HeaderMap,
) -> impl IntoResponse {
    info!("WS Upgrade Called");
    let my_bus = bus_tx.clone();
    let browser_language = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    ws.on_upgrade(move |socket| async {
        handle_socket(socket, my_bus, control_tx, browser_language).await;
    })
}

async fn handle_socket(
    mut socket: WebSocket,
    bus_tx: tokio::sync::mpsc::Sender<(
        tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
        lqos_bus::BusRequest,
    )>,
    control_tx: tokio::sync::mpsc::Sender<crate::lts2_sys::control_channel::ControlChannelCommand>,
    browser_language: Option<String>,
) {
    debug!("Websocket connected");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(10);
    let mut chatbot_request: Option<u64> = None;
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
                                    PrivateChannel::Chatbot { browser_ts_ms } => {
                                        // Start a chatbot session bridged via control channel
                                        if chatbot_request.is_none() {
                                            let request_id = rand::random::<u64>();
                                            chatbot_request = Some(request_id);
                                            let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
                                            let ctl = control_tx.clone();
                                            // Spawn forwarder: bytes -> text to client
                                            let to_client = tx.clone();
                                            tracing::info!("[chatbot] starting session request_id={} browser_ts_ms={:?}", request_id, browser_ts_ms);
                                            tokio::spawn(async move {
                                                while let Some(b) = stream_rx.recv().await {
                                                    let s = String::from_utf8_lossy(&b).to_string();
                                                    let _ = to_client.send(s).await;
                                                }
                                                tracing::info!("[chatbot] stream closed request_id={}", request_id);
                                            });
                                            // Send start to control channel
                                            let _ = ctl.send(
                                                crate::lts2_sys::control_channel::ControlChannelCommand::StartChat {
                                                    request_id,
                                                    browser_ts_ms,
                                                    browser_language: browser_language.clone(),
                                                    stream: stream_tx,
                                                }
                                            ).await;
                                        }
                                    },
                                }
                            } else {
                                // Try to parse chatbot user input when a session is active
                                if let Some(request_id) = chatbot_request {
                                    #[derive(Deserialize)]
                                    #[allow(non_snake_case)]
                                    struct ChatMsg { ChatbotUserInput: ChatMsgBody }
                                    #[derive(Deserialize)]
                                    struct ChatMsgBody { text: String }
                                    if let Ok(m) = serde_json::from_str::<ChatMsg>(text) {
                                        let _ = control_tx.send(
                                            crate::lts2_sys::control_channel::ControlChannelCommand::ChatSend {
                                                request_id,
                                                text: m.ChatbotUserInput.text,
                                            }
                                        ).await;
                                        tracing::debug!("[chatbot] forwarded user input request_id={}", request_id);
                                    }
                                } else {
                                    debug!("Failed to parse private message: {:?}", text);
                                }
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
