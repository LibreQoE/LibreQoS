mod cake_watcher;
mod chatbot;
pub(crate) mod circuit;
mod circuit_metrics;
pub(crate) mod flows_by_circuit;
mod ping_monitor;
mod tree_attached_circuits;

use crate::node_manager::ws::messages::{PrivateRequest, WsResponse, encode_ws_message};
use crate::node_manager::ws::single_user_channels::cake_watcher::cake_watcher;
use crate::node_manager::ws::single_user_channels::circuit::circuit_watcher;
use crate::node_manager::ws::single_user_channels::circuit_metrics::watch_circuit_metrics;
use crate::node_manager::ws::single_user_channels::ping_monitor::ping_monitor;
use crate::node_manager::ws::single_user_channels::tree_attached_circuits::watch_tree_attached_circuits;
use lqos_probe::ProbeClient;
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
    probe_client: ProbeClient,
    browser_language: Option<String>,
    chatbot_request: Option<u64>,
    circuit_watch: Option<tokio::task::JoinHandle<()>>,
    ping_monitor_watch: Option<tokio::task::JoinHandle<()>>,
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
        control_tx: tokio::sync::mpsc::Sender<
            crate::lts2_sys::control_channel::ControlChannelCommand,
        >,
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
            tree_attached_circuits_watch: None,
            circuit_metrics_watch: None,
        }
    }

    pub fn control_tx(
        &self,
    ) -> tokio::sync::mpsc::Sender<crate::lts2_sys::control_channel::ControlChannelCommand> {
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
                spawn(cake_watcher(circuit, self.tx.clone()));
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
            .send(
                crate::lts2_sys::control_channel::ControlChannelCommand::ChatSend {
                    request_id,
                    text,
                },
            )
            .await;
    }
}

impl Drop for PrivateState {
    fn drop(&mut self) {
        self.abort_circuit_watch();
        self.abort_ping_monitor_watch();
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
