use std::sync::Arc;
use axum::{extract::{ws::{Message, WebSocket}, WebSocketUpgrade}, response::IntoResponse, routing::get, Extension, Router};
use lqos_config::{load_config, Config};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{mpsc::Sender, Mutex};

use crate::throughput_tracker::THROUGHPUT_TRACKER;

pub fn websocket_router() -> Router {
    let channels = PubSub::new();
    tokio::spawn(channel_ticker(channels.clone()));
    Router::new()
        .route("/ws", get(ws_handler))
        .layer(Extension(channels))
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum Channel {
    Throughput,
}

impl Channel {
    fn as_str(&self) -> &'static str {
        match self {
            Channel::Throughput => "throughput",
        }
    }
}

async fn channel_ticker(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
    loop {
        interval.tick().await; // Once per second

        // Throughput Data
        let (bits_per_second, packets_per_second, shaped_bits_per_second) = {
            (
                THROUGHPUT_TRACKER.bits_per_second(),
                THROUGHPUT_TRACKER.packets_per_second(),
                THROUGHPUT_TRACKER.shaped_bits_per_second(),
            )
        };
        let max = if let Ok(config) = load_config() {
            (
                config.queues.uplink_bandwidth_mbps,
                config.queues.downlink_bandwidth_mbps,
            )
        } else {
            (0,0)
        };
        let bps = json!(
        {
            "event" : "throughput",
            "data": {
                "bps": bits_per_second,
                "pps": packets_per_second,
                "shaped_bps": shaped_bits_per_second,
                "max": max,
            }
        }
        ).to_string();
        channels.send_and_clean(Channel::Throughput, bps).await;
    }
}

struct PubSub {
    // This is a placeholder for a real pubsub system
    subs: Mutex<Vec<(bool, Channel, Sender<String>)>>,
}

impl PubSub {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            subs: Mutex::new(Vec::new()),
        })
    }

    async fn do_subscribe(&self, channel: Channel, tx: Sender<String>) {
        self.subs.lock().await.push((true, channel, tx.clone()));
        let welcome = json!(
            {
                "event" : "join",
                "channel" : channel.as_str(),
            }
        ).to_string();
        let _ = tx.send(welcome).await;
    }

    pub async fn subscribe(&self, channel: String, tx: Sender<String>) {
        match channel.as_str() {
            "throughput" => self.do_subscribe(Channel::Throughput, tx).await,
            _ => log::warn!("Unknown channel: {}", channel),
        }
    }

    async fn send_and_clean(&self, target_channel: Channel, message: String) {
        let mut subs = self.subs.lock().await;
        for (active, channel, tx) in subs.iter_mut() {
            if target_channel == *channel {
                if tx.send(message.clone()).await.is_err() {
                    *active = false;
                }            
            }
        }
        subs.retain(|(active, _, _)| *active);
    }
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
                                channels.subscribe(sub.channel, tx.clone()).await;
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