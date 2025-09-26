use anyhow::{Result, bail};
use futures_util::{StreamExt, sink::SinkExt};
use std::{collections::HashMap, time::Duration};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio::sync::mpsc::error::TrySendError;
use serde::{Serialize, Deserialize};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, warn};
use tungstenite::Message;

use crate::lts2_sys::lts2_client::{LicenseStatus, set_license_status};

mod messages;
pub use messages::{RemoteInsightRequest, WsMessage};

#[derive(Debug)]
pub struct HistoryQueryResultPayload {
    pub tag: String,
    pub seconds: i32,
    pub data: Option<Vec<u8>>,
}

pub enum ControlChannelCommand {
    SubmitChunks {
        serial: usize,
        chunks: Vec<Vec<u8>>,
    },
    FetchHistory {
        request: messages::RemoteInsightRequest,
        responder: oneshot::Sender<Result<HistoryQueryResultPayload, ()>>,
    },
}

pub enum ConnectionCommand {
    SubmitChunks {
        serial: usize,
        chunks: Vec<Vec<u8>>,
    },
    FetchHistory {
        request: messages::RemoteInsightRequest,
        responder: oneshot::Sender<Result<HistoryQueryResultPayload, ()>>,
    },
}

pub struct ControlChannelBuilder {
    pub tx: tokio::sync::mpsc::Sender<ControlChannelCommand>,
    rx: tokio::sync::mpsc::Receiver<ControlChannelCommand>,
}

pub fn init_control_channel() -> Result<ControlChannelBuilder> {
    // Doing this two-step: make the channel here and then spawn the task
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    Ok(ControlChannelBuilder { tx, rx })
}

pub async fn start_control_channel(builder: ControlChannelBuilder) -> Result<()> {
    tokio::spawn(async move {
        if let Err(e) = control_channel_loop(builder).await {
            tracing::error!("Control channel loop failed: {:?}", e);
        }
    });
    Ok(())
}

async fn control_channel_loop(mut builder: ControlChannelBuilder) -> Result<()> {
    // Handle the persistent channel to Insight here
    let (tx, rx) = tokio::sync::mpsc::channel::<ConnectionCommand>(1024);
    tokio::spawn(persistent_connection(rx));

    while let Some(cmd) = builder.rx.recv().await {
        match cmd {
            ControlChannelCommand::SubmitChunks { serial, chunks } => {
                let _ = tx.try_send(ConnectionCommand::SubmitChunks { serial, chunks });
            }
            ControlChannelCommand::FetchHistory { request, responder } => {
                if let Err(err) =
                    tx.try_send(ConnectionCommand::FetchHistory { request, responder })
                {
                    if let tokio::sync::mpsc::error::TrySendError::Full(
                        ConnectionCommand::FetchHistory { responder, .. },
                    )
                    | tokio::sync::mpsc::error::TrySendError::Closed(
                        ConnectionCommand::FetchHistory { responder, .. },
                    ) = err
                    {
                        let _ = responder.send(Err(()));
                    }
                }
            }
        }
    }
    warn!("Control channel loop exiting");
    Ok(())
}

const TCP_TIMEOUT: Duration = Duration::from_secs(10);

async fn persistent_connection(
    mut rx: tokio::sync::mpsc::Receiver<ConnectionCommand>,
) -> std::result::Result<(), String> {
    let mut sleep_seconds = 60;
    'reconnect: loop {
        if let Ok(mut socket) = connect().await {
            let mut permitted = false;
            // Preamble - get connected
            if let Err(e) = send_magic_number(&mut socket).await {
                warn!("Failed to send magic number: {}", e);
                tokio::time::sleep(Duration::from_secs(sleep_seconds)).await;
                continue 'reconnect;
            }
            if let Err(e) = send_license(&mut socket).await {
                warn!("Failed to send license info: {}", e);
                tokio::time::sleep(Duration::from_secs(sleep_seconds)).await;
                continue 'reconnect;
            }

            // Split the socket
            let (mut write, mut read) = socket.split();
            let (socket_sender_tx, mut socket_sender_rx) =
                tokio::sync::mpsc::channel::<Message>(32);
            let mut ping_interval = tokio::time::interval(Duration::from_secs(10));
            let mut license_interval = tokio::time::interval(Duration::from_secs(60 * 15)); // 15 minutes
            let mut pending_history: HashMap<
                u64,
                oneshot::Sender<Result<HistoryQueryResultPayload, ()>>,
            > = HashMap::new();
            let mut next_history_request_id: u64 = 1;

            // Message pump
            'message_pump: loop {
                // Inbound Message
                tokio::select! {
                    command = rx.recv() => {
                        debug!("Got command");
                        match command {
                            Some(ConnectionCommand::SubmitChunks { serial, chunks }) => {
                                if !permitted {
                                    warn!("Not permitted to send chunks yet");
                                    continue 'message_pump;
                                }
                                let n_chunks = chunks.len();
                                let byte_count = chunks.iter().map(|c| c.len()).sum::<usize>();

                                // Send BeginIngest
                                let Ok((_, _, bytes)) = messages::WsMessage::BeginIngest { unique_id: serial as u64, n_chunks: n_chunks as u64 }.to_bytes() else {
                                    error!("Failed to serialize BeginIngest message");
                                    break 'message_pump;
                                };
                                if let Err(e) = socket_sender_tx.try_send(Message::Binary(bytes.into())) {
                                    match e {
                                        TrySendError::Full(_) => {
                                            warn!("Send unavailable: BeginIngest queue full; dropping message");
                                        }
                                        TrySendError::Closed(_) => {
                                            error!("Failed to send BeginIngest message: channel closed");
                                            break 'message_pump;
                                        }
                                    }
                                }

                                // Submit Each Chunk
                                for (i, chunk) in chunks.into_iter().enumerate() {
                                    let Ok((_, _, bytes)) = messages::WsMessage::IngestChunk { unique_id: serial as u64, chunk: i as u64, n_chunks: n_chunks as u64, data: chunk }.to_bytes() else {
                                        error!("Failed to serialize IngestChunk message");
                                        break 'message_pump;
                                    };
                                    if let Err(e) = socket_sender_tx.try_send(Message::Binary(bytes.into())) {
                                        match e {
                                            TrySendError::Full(_) => {
                                                warn!("Send unavailable: IngestChunk queue full; dropping chunk");
                                            }
                                            TrySendError::Closed(_) => {
                                                error!("Failed to send IngestChunk message: channel closed");
                                                break 'message_pump;
                                            }
                                        }
                                    }
                                }

                                // Send EndIngest
                                let Ok((_, _, bytes)) = messages::WsMessage::EndIngest { unique_id: serial as u64, n_chunks: n_chunks as u64 }.to_bytes() else {
                                    error!("Failed to serialize EndIngest message");
                                    break 'message_pump;
                                };
                                if let Err(e) = socket_sender_tx.try_send(Message::Binary(bytes.into())) {
                                    match e {
                                        TrySendError::Full(_) => {
                                            warn!("Send unavailable: EndIngest queue full; dropping message");
                                        }
                                        TrySendError::Closed(_) => {
                                            error!("Failed to send EndIngest message: channel closed");
                                            break 'message_pump;
                                        }
                                    }
                                }
                                debug!("Submitted {} bytes for ingestion", byte_count);
                            }
                            Some(ConnectionCommand::FetchHistory { request, responder }) => {
                                if !permitted {
                                    warn!("Not permitted to request history yet");
                                    let _ = responder.send(Err(()));
                                    continue 'message_pump;
                                }

                                let request_id = next_history_request_id;
                                next_history_request_id = next_history_request_id.wrapping_add(1);
                                if pending_history.insert(request_id, responder).is_some() {
                                    warn!("Duplicate pending history request id {request_id}");
                                }

                                let message = messages::WsMessage::HistoryQuery {
                                    request_id,
                                    query: request,
                                };
                                let Ok((_, _, bytes)) = message.to_bytes() else {
                                    error!("Failed to serialize history query");
                                    if let Some(responder) = pending_history.remove(&request_id) {
                                        let _ = responder.send(Err(()));
                                    }
                                    continue 'message_pump;
                                };
                                if let Err(e) = socket_sender_tx.try_send(Message::Binary(bytes.into())) {
                                    match e {
                                        TrySendError::Full(_) => {
                                            warn!("Send unavailable: history query queue full; dropping request");
                                            if let Some(responder) = pending_history.remove(&request_id) {
                                                let _ = responder.send(Err(()));
                                            }
                                        }
                                        TrySendError::Closed(_) => {
                                            error!("Failed to queue history query: channel closed");
                                            if let Some(responder) = pending_history.remove(&request_id) {
                                                let _ = responder.send(Err(()));
                                            }
                                            break 'message_pump;
                                        }
                                    }
                                }
                            }
                            None => {
                                // Channel closed
                                error!("Command channel closed");
                                break 'message_pump;
                            }
                        }
                    }
                    message = timeout(TCP_TIMEOUT, read.next()) => {
                        let Ok(Some(Ok(message))) = message else {
                            // Timeout hit
                            error!("Timeout or read error on WSS read");
                            break 'message_pump;
                        };
                        match message {
                            Message::Binary(bytes) => {
                                // Actual message
                                let Ok(msg) = messages::WsMessage::from_bytes(&bytes) else {
                                    error!("Failed to parse incoming message");
                                    break 'message_pump;
                                };
                                // TODO: Handle incoming messages here
                                match msg {
                                    messages::WsMessage::Welcome { valid: _, license_state, expiration_date } => {
                                        debug!("Control channel connected and permitted");
                                        set_license_status(LicenseStatus {
                                            license_type: license_state,
                                            trial_expires: expiration_date as i32,
                                        });
                                        permitted = true;
                                        sleep_seconds = 60;
                                    }
                                    messages::WsMessage::Heartbeat { timestamp } => {
                                        // Send a heartbeat reply
                                        debug!("Received heartbeat, sending reply");
                                        let message = messages::WsMessage::HeartbeatReply { insight_time: timestamp };
                                        let Ok((_, _, bytes)) = message.to_bytes() else {
                                            error!("Failed to serialize heartbeat reply");
                                            break 'message_pump;
                                        };
                                        if let Err(e) = socket_sender_tx.try_send(Message::Binary(bytes.into())) {
                                            match e {
                                                TrySendError::Full(_) => {
                                                    warn!("Send unavailable: heartbeat reply queue full; dropping reply");
                                                }
                                                TrySendError::Closed(_) => {
                                                    error!("Failed to send heartbeat reply: channel closed");
                                                    break 'message_pump;
                                                }
                                            }
                                        }
                                    }
                                    messages::WsMessage::RemoteCommands { commands } => {
                                        crate::lts2_sys::lts2_client::enqueue(commands);
                                    }
                                    messages::WsMessage::HistoryQueryResult { request_id, tag, seconds, data } => {
                                        if let Some(responder) = pending_history.remove(&request_id) {
                                            let payload = HistoryQueryResultPayload { tag, seconds, data };
                                            let _ = responder.send(Ok(payload));
                                        } else {
                                            warn!("History query result for unknown request id {request_id}");
                                        }
                                    }
                                    messages::WsMessage::MakeApiRequest { request_id, method, url_suffix, body } => {
                                        let socket_sender_tx = socket_sender_tx.clone();
                                        tokio::spawn(async move {
                                            let Ok(()) = api_request(request_id, method, url_suffix, body, socket_sender_tx).await else {
                                                error!("API request handling failed");
                                                return;
                                            };
                                        });
                                    }
                                    messages::WsMessage::StartStreaming { request_id, circuit_hash } => {
                                        let socket_sender_tx = socket_sender_tx.clone();
                                        tokio::spawn(async move {
                                            let Ok(()) = circuit_snapshot_streaming(request_id, circuit_hash, socket_sender_tx).await else {
                                                error!("Circuit snapshot streaming failed");
                                                return;
                                            };
                                        });
                                    }
                                    _ => {}
                                }
                            }
                            Message::Text(..) => {
                                error!("We shouldn't receive a text message");
                                break 'message_pump;
                            }
                            Message::Ping(payload) => {
                                // Actual message - good
                                if let Err(e) = socket_sender_tx.try_send(Message::Pong(payload)) {
                                    match e {
                                        TrySendError::Full(_) => {
                                            warn!("Send unavailable: pong queue full; dropping pong");
                                        }
                                        TrySendError::Closed(_) => {
                                            error!("Failed to send Pong message: channel closed");
                                            break 'message_pump;
                                        }
                                    }
                                }
                            }
                            Message::Pong(..) => {
                                // Actual message - good
                            }
                            Message::Close(..) => {
                                debug!("WebSocket closed by remote");
                                break 'message_pump;
                            }
                            Message::Frame(..) => {} // Shouldn't happen
                        }
                    }
                    outbound = socket_sender_rx.recv() => {
                        let Some(outbound) = outbound else {
                            error!("Outbound message pump stopped");
                            break 'message_pump;
                        };
                        let Ok(Ok(_)) = timeout(TCP_TIMEOUT, write.send(outbound)).await else {
                            // Outbound sending is failing
                            error!("Failed to send outbound message");
                            break 'message_pump;
                        };
                    }
                    _ = ping_interval.tick() => {
                        // Send a WsMessage::Ping
                        debug!("Sending Ping message");
                        let bytes = vec![1u8; 4];
                        if let Err(e) = socket_sender_tx.try_send(Message::Ping(bytes.into())) {
                            match e {
                                TrySendError::Full(_) => {
                                    warn!("Send unavailable: ping queue full; dropping ping");
                                }
                                TrySendError::Closed(_) => {
                                    error!("Failed to send Ping message: channel closed");
                                    break 'message_pump;
                                }
                            }
                        }
                    }
                    _ = license_interval.tick() => {
                        let Ok(config) = lqos_config::load_config() else {
                            break 'message_pump;
                        };
                        let Some(license) = &config.long_term_stats.license_key else {
                            break 'message_pump;
                        };
                        let Ok(license) = license.replace("-", "").parse::<uuid::Uuid>() else {
                            break 'message_pump;
                        };

                        let message = messages::WsMessage::License {
                            license,
                            node_id: config.node_id.clone(),
                            node_name: config.node_name.clone(),
                        };
                        let Ok((_, _, bytes)) = message.to_bytes() else {
                            error!("Failed to serialize license message");
                            break 'message_pump;
                        };
                        if let Err(e) = socket_sender_tx.try_send(Message::Binary(bytes.into())) {
                            match e {
                                TrySendError::Full(_) => {
                                    warn!("Send unavailable: license message queue full; dropping message");
                                }
                                TrySendError::Closed(_) => {
                                    error!("Failed to send license message: channel closed");
                                    break 'message_pump;
                                }
                            }
                        }
                    }
                }
            } // End of message pump
            for (_, responder) in pending_history.drain() {
                let _ = responder.send(Err(()));
            }
        }
        debug!("Sleeping before reconnecting the persistent channel");
        tokio::time::sleep(Duration::from_secs(sleep_seconds)).await;
        sleep_seconds = 60;
    }
}

async fn connect() -> anyhow::Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
    let remote_host = crate::lts2_sys::lts2_client::get_remote_host();
    let target = format!("wss://{}:443/shaper_gateway/ws", remote_host);
    debug!("Connecting to shaper gateway: {target}");

    // DNS resolution with timeout
    let lookup = timeout(
        TCP_TIMEOUT,
        tokio::net::lookup_host((remote_host.as_str(), 443)),
    )
    .await;
    let Ok(Ok(mut addresses)) = lookup else {
        warn!("DNS resolution failed or timed out for host: {remote_host}");
        bail!("DNS Error");
    };
    let Some(addr) = addresses.next() else {
        bail!("DNS Error");
    };

    // TCP Stream
    let Ok(Ok(stream)) = timeout(TCP_TIMEOUT, TcpStream::connect(&addr)).await else {
        warn!("Failed to connect to shaper gateway server: {remote_host}");
        bail!("Failed to connect to shaper gateway server".to_string());
    };

    // Native TLS
    debug!("Connected to shaper gateway server: {remote_host}");
    let Ok(connector) = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
    else {
        warn!("Failed to create TLS connector");
        bail!("Failed to create TLS connector");
    };
    let t_connector = tokio_tungstenite::Connector::NativeTls(connector);

    // Tungstenite Client
    debug!("Connecting tungstenite client to shaper gateway server: {target}");
    let result = timeout(
        TCP_TIMEOUT,
        tokio_tungstenite::client_async_tls_with_config(target, stream, None, Some(t_connector)),
    )
    .await;
    if result.is_err() {
        bail!("Failed to connect to shaper gateway server. {result:?}");
    }
    let Ok(Ok((socket, _response))) = result else {
        warn!("Failed to connect to shaper gateway server {result:?}");
        bail!("Failed to connect to shaper gateway server. {result:?}");
    };
    debug!("Connected");

    Ok(socket)
}

async fn send_magic_number(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> anyhow::Result<()> {
    const MAGIC_NUMBER: u32 = 0x8123a;
    let message = messages::WsMessage::Hello {
        magic: MAGIC_NUMBER,
    };
    let (_raw_size, _compressed_size, bytes) = message.to_bytes()?;
    timeout(
        TCP_TIMEOUT,
        socket.send(tokio_tungstenite::tungstenite::Message::Binary(
            bytes.into(),
        )),
    )
    .await??;
    Ok(())
}

async fn send_license(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> anyhow::Result<()> {
    let Ok(config) = lqos_config::load_config() else {
        bail!("Failed to load config");
    };
    let Some(license) = &config.long_term_stats.license_key else {
        bail!("No license key found");
    };
    let Ok(license) = license.replace("-", "").parse::<uuid::Uuid>() else {
        bail!("Invalid license key format");
    };

    let message = messages::WsMessage::License {
        license,
        node_id: config.node_id.clone(),
        node_name: config.node_name.clone(),
    };
    let (_, _, bytes) = message.to_bytes()?;
    timeout(
        TCP_TIMEOUT,
        socket.send(tokio_tungstenite::tungstenite::Message::Binary(
            bytes.into(),
        )),
    )
    .await??;

    Ok(())
}

async fn api_request(
    request_id: u64,
    method: messages::ApiRequestType,
    url_suffix: String,
    body: Option<String>,
    reply: tokio::sync::mpsc::Sender<Message>,
) -> anyhow::Result<()> {
    // Make a request to http://127.0.0.1:9122/{url_suffix} using the specified method and body (if present),
    // and return the result (status, headers, body) to Insight as an ApiReply.
    let client = reqwest::Client::builder()
        .timeout(TCP_TIMEOUT)
        .build()?;

    let path = url_suffix.trim_start_matches('/');
    let url = format!("http://127.0.0.1:9122/{}", path);

    // Load license key for x-bearer header
    let bearer = match lqos_config::load_config() {
        Ok(cfg) => {
            if let Some(key) = cfg.long_term_stats.license_key.as_ref() {
                key.replace('-', "")
            } else {
                warn!("No license key found for API pass-through; x-bearer not set");
                String::new()
            }
        }
        Err(e) => {
            warn!("Failed to load config for API pass-through: {}", e);
            String::new()
        }
    };

    let mut req = match method {
        messages::ApiRequestType::Get => client.get(&url),
        messages::ApiRequestType::Post => client.post(&url),
        messages::ApiRequestType::Delete => client.delete(&url),
    };

    if !bearer.is_empty() {
        req = req.header("x-bearer", bearer);
    }

    if let Some(b) = body {
        req = req.body(b);
    }

    let (status, headers, bytes) = match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let headers: Vec<(String, String)> = resp
                .headers()
                .iter()
                .filter_map(|(k, v)| match v.to_str() {
                    Ok(val) => Some((k.as_str().to_string(), val.to_string())),
                    Err(_) => None,
                })
                .collect();
            match resp.bytes().await {
                Ok(b) => (status, headers, b.to_vec()),
                Err(e) => {
                    warn!("API daemon response read failed: {}", e);
                    (status, headers, Vec::new())
                }
            }
        }
        Err(e) => {
            warn!("API daemon request failed: {}", e);
            (0u16, Vec::new(), Vec::new())
        }
    };

    let message = messages::WsMessage::ApiReply { request_id, status, headers, data: bytes };
    let Ok((_, _, ws_bytes)) = message.to_bytes() else {
        error!("Failed to serialize ApiReply");
        return Ok(());
    };

    if let Err(e) = reply.try_send(Message::Binary(ws_bytes.into())) {
        match e {
            TrySendError::Full(_) => {
                warn!("Send unavailable: ApiReply queue full; dropping reply");
            }
            TrySendError::Closed(_) => {
                error!("Failed to send ApiReply: channel closed");
            }
        }
    }

    Ok(())
}

async fn circuit_snapshot_streaming(
    request_id: u64,
    circuit_hash: i64,
    reply: tokio::sync::mpsc::Sender<Message>,
) -> anyhow::Result<()> {
    #[derive(Serialize, Deserialize)]
    struct DeviceSnapshot {
        device_id: String,
        device_name: String,
        addresses: Vec<(std::net::IpAddr, u8)>,
        last_ping_ms: Option<f32>,
        // down, up bits per second
        bits_per_second: (u64, u64),
        median_tcp_rtt_ms: Option<f32>,
        // down, up retransmit percentage
        tcp_retransmit_pct: (f64, f64),
    }

    #[derive(Serialize, Deserialize)]
    struct FlowSnapshot {
        remote_ip: std::net::IpAddr,
        local_ip: std::net::IpAddr,
        src_port: u16,
        dst_port: u16,
        ip_protocol: u8,
        last_seen_nanos: u64,
        // Down/Up tuples for compact transport
        rate_bps: (u32, u32),
        bytes_sent: (u64, u64),
        packets_sent: (u64, u64),
        tcp_retransmits: (u16, u16),
        rtt_ms: (f32, f32),
    }

    #[derive(Serialize, Deserialize)]
    struct StreamingCircuitPayload {
        devices: Vec<DeviceSnapshot>,
        flows: Vec<FlowSnapshot>,
    }

    // Helper: choose a host IP for ping (only /32 or /128)
    fn choose_host_ip(
        v4: &[(std::net::Ipv4Addr, u32)],
        v6: &[(std::net::Ipv6Addr, u32)],
    ) -> Option<std::net::IpAddr> {
        if let Some((ip, plen)) = v4.iter().find(|(_, p)| *p == 32) {
            return Some(std::net::IpAddr::V4(*ip));
        }
        if let Some((ip, plen)) = v6.iter().find(|(_, p)| *p == 128) {
            return Some(std::net::IpAddr::V6(*ip));
        }
        None
    }

    // Helper: run one ping with 1s timeout; respects disable_icmp_ping
    async fn one_ping(ip: std::net::IpAddr) -> Option<f32> {
        let Ok(cfg) = lqos_config::load_config() else { return None; };
        if cfg.disable_icmp_ping.unwrap_or(false) {
            return None;
        }
        use surge_ping::{Client, Config, ICMP, IcmpPacket, PingIdentifier, PingSequence};
        use rand::random;
        let client = match ip {
            std::net::IpAddr::V4(_) => Client::new(&Config::default()),
            std::net::IpAddr::V6(_) => Client::new(&Config::builder().kind(ICMP::V6).build()),
        };
        if client.is_err() { return None; }
        let client = client.ok()?;
        let payload = [0; 56];
        let mut pinger = client.pinger(ip, PingIdentifier(random())).await;
        pinger.timeout(Duration::from_secs(1));
        match pinger.ping(PingSequence(0), &payload).await {
            Ok((IcmpPacket::V4(..), dur)) | Ok((IcmpPacket::V6(..), dur)) => Some(dur.as_secs_f32() * 1000.0),
            _ => None,
        }
    }

    // Load shaped devices snapshot and pick devices in the target circuit
    let shaped = crate::shaped_devices_tracker::SHAPED_DEVICES.load();
    let mut device_indexes: Vec<usize> = Vec::new();
    for (idx, dev) in shaped.devices.iter().enumerate() {
        if dev.circuit_hash == circuit_hash {
            device_indexes.push(idx);
        }
    }

    // Aggregation holders per device index
    use lqos_utils::units::{DownUpOrder, down_up_divide};
    struct Agg {
        bps_bytes: DownUpOrder<u64>,
        tcp_packets: DownUpOrder<u64>,
        tcp_retries: DownUpOrder<u64>,
        rtts: Vec<f32>,
    }
    let mut aggregates: std::collections::HashMap<usize, Agg> = std::collections::HashMap::new();
    for idx in &device_indexes {
        aggregates.insert(*idx, Agg { bps_bytes: DownUpOrder::zeroed(), tcp_packets: DownUpOrder::zeroed(), tcp_retries: DownUpOrder::zeroed(), rtts: Vec::new() });
    }

    // Walk raw throughput data and fold into devices of this circuit
    let tp_cycle = crate::throughput_tracker::THROUGHPUT_TRACKER
        .cycle
        .load(std::sync::atomic::Ordering::Relaxed);
    {
        let raw = crate::throughput_tracker::THROUGHPUT_TRACKER.raw_data.lock();
        for (xdp_ip, te) in raw.iter() {
            // Only consider entries known to belong to this circuit and are fresh enough
            if te.circuit_hash != Some(circuit_hash) { continue; }
            // retire_check is local; use same heuristic: require most_recent_cycle >= tp_cycle - RETIRE_AFTER_SECONDS
            // We don't have RETIRE_AFTER_SECONDS here; accept all entries for snapshot.
            // Map IP to device via trie
            let lookup = xdp_ip.as_ipv6();
            if let Some((_, id)) = shaped.trie.longest_match(lookup) {
                if aggregates.contains_key(id) {
                    if let Some(agg) = aggregates.get_mut(id) {
                        // bytes_per_second -> later convert to bits
                        agg.bps_bytes += te.bytes_per_second;
                        agg.tcp_packets += te.tcp_packets;
                        agg.tcp_retries += te.tcp_retransmits;
                        if let Some(rtt) = te.median_latency() {
                            agg.rtts.push(rtt);
                        }
                    }
                }
            }
        }
        // raw lock dropped here
    }

    // Build device snapshots
    let mut devices_out: Vec<DeviceSnapshot> = Vec::new();
    for idx in device_indexes {
        let dev = &shaped.devices[idx];
        let agg = aggregates.remove(&idx).unwrap_or(Agg { bps_bytes: DownUpOrder::zeroed(), tcp_packets: DownUpOrder::zeroed(), tcp_retries: DownUpOrder::zeroed(), rtts: Vec::new() });
        // Compact addresses into a single list
        let mut addresses: Vec<(std::net::IpAddr, u8)> = Vec::new();
        addresses.extend(
            dev.ipv4
                .iter()
                .map(|(ip, p)| (std::net::IpAddr::V4(*ip), *p as u8)),
        );
        addresses.extend(
            dev.ipv6
                .iter()
                .map(|(ip, p)| (std::net::IpAddr::V6(*ip), *p as u8)),
        );
        // Prepare values BEFORE await to avoid holding references across .await
        let device_id = dev.device_id.clone();
        let device_name = dev.device_name.clone();
        // Ping selection
        let ping_ip = choose_host_ip(&dev.ipv4, &dev.ipv6);
        let last_ping_ms = if let Some(ip) = ping_ip { one_ping(ip).await } else { None };
        // Compute RTT median across collected medians
        let median_tcp_rtt_ms = if agg.rtts.is_empty() {
            None
        } else {
            let mut r = agg.rtts.clone();
            r.sort_by(|a, b| a.partial_cmp(b).unwrap());
            Some(r[r.len() / 2])
        };
        // Compute retransmit percentage
        let tcp_retransmit_pct = if agg.tcp_packets.down == 0 && agg.tcp_packets.up == 0 {
            (0.0, 0.0)
        } else {
            down_up_divide(agg.tcp_retries, agg.tcp_packets)
        };
        // Bits per second (down, up)
        let bits_per_second = (
            agg.bps_bytes.down.saturating_mul(8),
            agg.bps_bytes.up.saturating_mul(8),
        );

        devices_out.push(DeviceSnapshot {
            device_id,
            device_name,
            addresses,
            last_ping_ms,
            bits_per_second,
            median_tcp_rtt_ms,
            tcp_retransmit_pct,
        });
    }

    // Build flow snapshots for this circuit (recent, last 5 minutes)
    let mut flows_out: Vec<FlowSnapshot> = Vec::new();
    if let Ok(now_ts) = lqos_utils::unix_time::time_since_boot() {
        let now_nanos = std::time::Duration::from(now_ts).as_nanos() as u64;
        let five_minutes_ago = now_nanos.saturating_sub(300 * 1_000_000_000);
        let all_flows = crate::throughput_tracker::flow_data::ALL_FLOWS.lock();
        for (key, (local, analysis)) in all_flows.flow_data.iter() {
            if local.last_seen < five_minutes_ago { continue; }
            // Identify if this flow belongs to our circuit via local or remote mapping
            use std::net::IpAddr;
            let local_ip_v6 = match key.local_ip.as_ip() { IpAddr::V4(ip) => ip.to_ipv6_mapped(), IpAddr::V6(ip) => ip };
            let remote_ip_v6 = match key.remote_ip.as_ip() { IpAddr::V4(ip) => ip.to_ipv6_mapped(), IpAddr::V6(ip) => ip };
            let mut include = false;
            let (mut local_ip_addr, mut remote_ip_addr) = (key.local_ip.as_ip(), key.remote_ip.as_ip());
            if let Some((_, id)) = shaped.trie.longest_match(local_ip_v6) {
                if shaped.devices[*id].circuit_hash == circuit_hash {
                    include = true;
                    local_ip_addr = key.local_ip.as_ip();
                    remote_ip_addr = key.remote_ip.as_ip();
                }
            }
            if !include {
                if let Some((_, id)) = shaped.trie.longest_match(remote_ip_v6) {
                    if shaped.devices[*id].circuit_hash == circuit_hash {
                        include = true;
                        // swap orientation so local is on-circuit side
                        local_ip_addr = key.remote_ip.as_ip();
                        remote_ip_addr = key.local_ip.as_ip();
                    }
                }
            }
            if !include { continue; }

            let rtt_ms = (
                local.rtt[0].as_millis() as f32,
                local.rtt[1].as_millis() as f32,
            );
            flows_out.push(FlowSnapshot {
                remote_ip: remote_ip_addr,
                local_ip: local_ip_addr,
                src_port: key.src_port,
                dst_port: key.dst_port,
                ip_protocol: key.ip_protocol,
                last_seen_nanos: now_nanos.saturating_sub(local.last_seen),
                rate_bps: (local.rate_estimate_bps.down, local.rate_estimate_bps.up),
                bytes_sent: (local.bytes_sent.down, local.bytes_sent.up),
                packets_sent: (local.packets_sent.down, local.packets_sent.up),
                tcp_retransmits: (local.tcp_retransmits.down, local.tcp_retransmits.up),
                rtt_ms,
            });
        }
    }

    let payload = StreamingCircuitPayload { devices: devices_out, flows: flows_out };
    let Ok(bytes) = serde_cbor::to_vec(&payload) else {
        error!("Failed to serialize StreamingCircuitPayload");
        return Ok(());
    };

    let message = messages::WsMessage::StreamingCircuit { request_id, circuit_hash, data: bytes };
    let Ok((_, _, ws_bytes)) = message.to_bytes() else {
        error!("Failed to serialize StreamingCircuit message");
        return Ok(());
    };
    if let Err(e) = reply.try_send(Message::Binary(ws_bytes.into())) {
        match e {
            TrySendError::Full(_) => warn!("Send unavailable: StreamingCircuit queue full; dropping reply"),
            TrySendError::Closed(_) => error!("Failed to send StreamingCircuit: channel closed"),
        }
    }
    Ok(())
}
