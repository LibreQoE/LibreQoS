mod cake_watcher;
mod chatbot;
mod circuit;
mod flows_by_circuit;
mod ping_monitor;

use crate::node_manager::ws::messages::{PrivateRequest, WsResponse, encode_ws_message};
use crate::node_manager::ws::single_user_channels::cake_watcher::cake_watcher;
use crate::node_manager::ws::single_user_channels::circuit::circuit_watcher;
use crate::node_manager::ws::single_user_channels::flows_by_circuit::flows_by_circuit;
use crate::node_manager::ws::single_user_channels::ping_monitor::ping_monitor;
use tokio::spawn;
use tokio::sync::mpsc::Sender;
use tracing::info;

pub struct PrivateState {
    tx: Sender<std::sync::Arc<Vec<u8>>>,
    bus_tx: Sender<(
        tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
        lqos_bus::BusRequest,
    )>,
    control_tx: tokio::sync::mpsc::Sender<crate::lts2_sys::control_channel::ControlChannelCommand>,
    browser_language: Option<String>,
    chatbot_request: Option<u64>,
}

impl PrivateState {
    pub fn new(
        tx: Sender<std::sync::Arc<Vec<u8>>>,
        bus_tx: Sender<(
            tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
            lqos_bus::BusRequest,
        )>,
        control_tx: tokio::sync::mpsc::Sender<
            crate::lts2_sys::control_channel::ControlChannelCommand,
        >,
        browser_language: Option<String>,
    ) -> Self {
        Self {
            tx,
            bus_tx,
            control_tx,
            browser_language,
            chatbot_request: None,
        }
    }

    pub async fn handle_request(&mut self, request: PrivateRequest) {
        match request {
            PrivateRequest::CircuitWatcher { circuit } => {
                spawn(circuit_watcher(
                    circuit,
                    self.tx.clone(),
                    self.bus_tx.clone(),
                ));
            }
            PrivateRequest::PingMonitor { ips } => {
                spawn(ping_monitor(ips, self.tx.clone()));
            }
            PrivateRequest::FlowsByCircuit { circuit } => {
                spawn(flows_by_circuit(circuit, self.tx.clone()));
            }
            PrivateRequest::CakeWatcher { circuit } => {
                spawn(cake_watcher(circuit, self.tx.clone()));
            }
            PrivateRequest::Chatbot { browser_ts_ms } => {
                self.start_chatbot(normalize_browser_ts_ms(browser_ts_ms))
                    .await;
            }
            PrivateRequest::ChatbotUserInput { text } => {
                self.forward_chatbot_input(text).await;
            }
        }
    }

    async fn start_chatbot(&mut self, browser_ts_ms: Option<i64>) {
        if self.chatbot_request.is_some() {
            return;
        }

        let request_id = rand::random::<u64>();
        self.chatbot_request = Some(request_id);
        let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
        let to_client = self.tx.clone();

        tokio::spawn(async move {
            while let Some(chunk) = stream_rx.recv().await {
                let text = String::from_utf8_lossy(&chunk).to_string();
                let response = WsResponse::ChatbotChunk { text };
                if let Ok(payload) = encode_ws_message(&response) {
                    if to_client.send(payload).await.is_err() {
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        let _ = self
            .control_tx
            .send(
                crate::lts2_sys::control_channel::ControlChannelCommand::StartChat {
                    request_id,
                    browser_ts_ms,
                    browser_language: self.browser_language.clone(),
                    stream: stream_tx,
                },
            )
            .await;
        info!(
            "[chatbot] starting session request_id={} browser_ts_ms={:?}",
            request_id, browser_ts_ms
        );
    }

    async fn forward_chatbot_input(&self, text: String) {
        let Some(request_id) = self.chatbot_request else {
            return;
        };
        let _ = self
            .control_tx
            .send(crate::lts2_sys::control_channel::ControlChannelCommand::ChatSend {
                request_id,
                text,
            })
            .await;
    }
}

// JS CBOR encoder emits float64 for timestamps beyond 32-bit ranges; normalize to i64.
fn normalize_browser_ts_ms(browser_ts_ms: Option<f64>) -> Option<i64> {
    let ts_ms = browser_ts_ms?;
    if !ts_ms.is_finite() {
        return None;
    }
    let ts_ms = ts_ms.trunc();
    if ts_ms < i64::MIN as f64 || ts_ms > i64::MAX as f64 {
        return None;
    }
    Some(ts_ms as i64)
}
