mod cake_watcher;
mod chatbot;
pub(crate) mod circuit;
mod circuit_metrics;
pub(crate) mod flows_by_circuit;
mod ping_monitor;
mod tree_attached_circuits;

use crate::lts2_sys::control_channel::ControlChannelCommand;
use crate::node_manager::ws::messages::{PrivateRequest, WsResponse, encode_ws_message};
use crate::node_manager::ws::single_user_channels::cake_watcher::cake_watcher;
use crate::node_manager::ws::single_user_channels::circuit::circuit_watcher;
use crate::node_manager::ws::single_user_channels::circuit_metrics::watch_circuit_metrics;
use crate::node_manager::ws::single_user_channels::ping_monitor::ping_monitor;
use crate::node_manager::ws::single_user_channels::tree_attached_circuits::watch_tree_attached_circuits;
use lqos_probe::ProbeClient;
use std::{sync::Arc, time::Duration};
use tokio::spawn;
use tokio::sync::mpsc::{Sender, error::TrySendError};
use tokio::time::timeout;
use tracing::{debug, info};

const CONTROL_CHANNEL_SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// Sends a private watcher payload without waiting behind a slow websocket.
pub(super) fn try_send_private_payload(
    tx: &Sender<Arc<Vec<u8>>>,
    payload: Arc<Vec<u8>>,
    channel_name: &'static str,
) -> bool {
    match tx.try_send(payload) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            debug!("{channel_name} outbound queue full; stopping watcher");
            false
        }
        Err(TrySendError::Closed(_)) => {
            debug!("{channel_name} channel closed");
            false
        }
    }
}

async fn send_control_command(
    control_tx: &tokio::sync::mpsc::Sender<ControlChannelCommand>,
    command: ControlChannelCommand,
    context: &'static str,
) {
    match timeout(CONTROL_CHANNEL_SEND_TIMEOUT, control_tx.send(command)).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => debug!("{context} control channel is closed"),
        Err(_) => debug!("{context} timed out queueing control channel command"),
    }
}

pub struct PrivateState {
    tx: Sender<Arc<Vec<u8>>>,
    bus_tx: Sender<(
        tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
        lqos_bus::BusRequest,
    )>,
    control_tx: tokio::sync::mpsc::Sender<ControlChannelCommand>,
    probe_client: ProbeClient,
    browser_language: Option<String>,
    chatbot_request: Option<u64>,
    circuit_watch: Option<tokio::task::JoinHandle<()>>,
    ping_monitor_watch: Option<tokio::task::JoinHandle<()>>,
    cake_watch: Option<tokio::task::JoinHandle<()>>,
    tree_attached_circuits_watch: Option<tokio::task::JoinHandle<()>>,
    circuit_metrics_watch: Option<tokio::task::JoinHandle<()>>,
}

impl PrivateState {
    pub fn new(
        tx: Sender<std::sync::Arc<Vec<u8>>>,
        bus_tx: Sender<(
            tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
            lqos_bus::BusRequest,
        )>,
        control_tx: tokio::sync::mpsc::Sender<ControlChannelCommand>,
        probe_client: ProbeClient,
        browser_language: Option<String>,
    ) -> Self {
        Self {
            tx,
            bus_tx,
            control_tx,
            probe_client,
            browser_language,
            chatbot_request: None,
            circuit_watch: None,
            ping_monitor_watch: None,
            cake_watch: None,
            tree_attached_circuits_watch: None,
            circuit_metrics_watch: None,
        }
    }

    pub fn control_tx(&self) -> tokio::sync::mpsc::Sender<ControlChannelCommand> {
        self.control_tx.clone()
    }

    pub fn bus_tx(
        &self,
    ) -> Sender<(
        tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
        lqos_bus::BusRequest,
    )> {
        self.bus_tx.clone()
    }

    pub async fn handle_request(&mut self, request: PrivateRequest) {
        match request {
            PrivateRequest::CircuitWatcher { circuit } => {
                self.replace_circuit_watch(circuit);
            }
            PrivateRequest::PingMonitor { ips } => {
                self.replace_ping_monitor_watch(ips);
            }
            PrivateRequest::StopCircuitWatcher => {
                self.abort_circuit_watch();
            }
            PrivateRequest::StopPingMonitorWatch => {
                self.abort_ping_monitor_watch();
            }
            PrivateRequest::CakeWatcher { circuit } => {
                self.replace_cake_watch(circuit);
            }
            PrivateRequest::StopCakeWatcher => {
                self.abort_cake_watch();
            }
            PrivateRequest::Chatbot { browser_ts_ms } => {
                self.start_chatbot(normalize_browser_ts_ms(browser_ts_ms))
                    .await;
            }
            PrivateRequest::ChatbotUserInput { text } => {
                self.forward_chatbot_input(text).await;
            }
            PrivateRequest::WatchTreeAttachedCircuits { query } => {
                self.replace_tree_attached_circuits_watch(query);
            }
            PrivateRequest::StopTreeAttachedCircuitsWatch => {
                self.abort_tree_attached_circuits_watch();
            }
            PrivateRequest::WatchCircuitMetrics { query } => {
                self.replace_circuit_metrics_watch(query);
            }
            PrivateRequest::StopCircuitMetricsWatch => {
                self.abort_circuit_metrics_watch();
            }
        }
    }

    fn replace_circuit_watch(&mut self, circuit: String) {
        self.abort_circuit_watch();
        self.circuit_watch = Some(spawn(circuit_watcher(
            circuit,
            self.tx.clone(),
            self.bus_tx.clone(),
        )));
    }

    fn abort_circuit_watch(&mut self) {
        if let Some(handle) = self.circuit_watch.take() {
            handle.abort();
        }
    }

    fn replace_ping_monitor_watch(&mut self, ips: Vec<(String, String)>) {
        self.abort_ping_monitor_watch();
        self.ping_monitor_watch = Some(spawn(ping_monitor(
            ips,
            self.tx.clone(),
            self.probe_client.clone(),
        )));
    }

    fn abort_ping_monitor_watch(&mut self) {
        if let Some(handle) = self.ping_monitor_watch.take() {
            handle.abort();
        }
    }

    fn replace_cake_watch(&mut self, circuit: String) {
        self.abort_cake_watch();
        self.cake_watch = Some(spawn(cake_watcher(circuit, self.tx.clone())));
    }

    fn abort_cake_watch(&mut self) {
        if let Some(handle) = self.cake_watch.take() {
            handle.abort();
        }
    }

    fn replace_tree_attached_circuits_watch(
        &mut self,
        query: crate::node_manager::local_api::tree_attached_circuits::TreeAttachedCircuitsQuery,
    ) {
        self.abort_tree_attached_circuits_watch();
        self.tree_attached_circuits_watch =
            Some(spawn(watch_tree_attached_circuits(query, self.tx.clone())));
    }

    fn abort_tree_attached_circuits_watch(&mut self) {
        if let Some(handle) = self.tree_attached_circuits_watch.take() {
            handle.abort();
        }
    }

    fn replace_circuit_metrics_watch(
        &mut self,
        query: crate::node_manager::local_api::circuit_live::CircuitMetricsQuery,
    ) {
        self.abort_circuit_metrics_watch();
        self.circuit_metrics_watch = Some(spawn(watch_circuit_metrics(query, self.tx.clone())));
    }

    fn abort_circuit_metrics_watch(&mut self) {
        if let Some(handle) = self.circuit_metrics_watch.take() {
            handle.abort();
        }
    }

    async fn start_chatbot(&mut self, browser_ts_ms: Option<i64>) {
        if self.chatbot_request.is_some() {
            return;
        }

        let capabilities = crate::lts2_sys::current_capabilities();
        if !capabilities.can_use_chatbot || !capabilities.control_service_reachable {
            let message = if capabilities.can_use_chatbot {
                "license valid, control service unavailable"
            } else {
                "Libby requires an entitled license."
            };
            let response = WsResponse::Error {
                message: message.to_string(),
            };
            if let Ok(payload) = encode_ws_message(&response) {
                let _ = try_send_private_payload(&self.tx, payload, "Chatbot");
            }
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
                    if !try_send_private_payload(&to_client, payload, "Chatbot") {
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        send_control_command(
            &self.control_tx,
            ControlChannelCommand::StartChat {
                request_id,
                browser_ts_ms,
                browser_language: self.browser_language.clone(),
                stream: stream_tx,
            },
            "StartChat",
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
        send_control_command(
            &self.control_tx,
            ControlChannelCommand::ChatSend { request_id, text },
            "ChatSend",
        )
        .await;
    }
}

impl Drop for PrivateState {
    fn drop(&mut self) {
        self.abort_circuit_watch();
        self.abort_ping_monitor_watch();
        self.abort_cake_watch();
        self.abort_tree_attached_circuits_watch();
        self.abort_circuit_metrics_watch();
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
