use anyhow::{bail, Result};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{error, info, warn};
use tungstenite::Message;
use std::{net::ToSocketAddrs, time::Duration};
use futures_util::{sink::SinkExt, StreamExt};

use crate::lts2_sys::{lts2_client::{set_license_status, LicenseStatus}};

mod messages;

pub enum ControlChannelCommand {
    SubmitChunks { serial: usize, chunks: Vec<Vec<u8>> },
}

pub enum ConnectionCommand {
    SubmitChunks { serial: usize, chunks: Vec<Vec<u8>> },
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
        }
    }
    warn!("Control channel loop exiting");
    Ok(())
}

const TCP_TIMEOUT: Duration = Duration::from_secs(10);

async fn persistent_connection(mut rx: tokio::sync::mpsc::Receiver<ConnectionCommand>) -> std::result::Result<(), String> {

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
            let (socket_sender_tx, mut socket_sender_rx) = tokio::sync::mpsc::channel::<Message>(32);
            let mut ping_interval = tokio::time::interval(Duration::from_secs(10));
            let mut license_interval = tokio::time::interval(Duration::from_secs(60 * 15)); // 15 minutes

            // Message pump
            'message_pump: loop {
                // Inbound Message
                tokio::select! {
                    command = rx.recv() => {
                        info!("Got command");
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
                                if let Err(_) = socket_sender_tx.send(Message::Binary(bytes.into())).await {
                                    error!("Failed to send BeginIngest message");
                                    break 'message_pump;
                                }

                                // Submit Each Chunk
                                for (i, chunk) in chunks.into_iter().enumerate() {
                                    let Ok((_, _, bytes)) = messages::WsMessage::IngestChunk { unique_id: serial as u64, chunk: i as u64, n_chunks: n_chunks as u64, data: chunk }.to_bytes() else {
                                        error!("Failed to serialize IngestChunk message");
                                        break 'message_pump;
                                    };
                                    if let Err(_) = socket_sender_tx.send(Message::Binary(bytes.into())).await {
                                        error!("Failed to send IngestChunk message");
                                        break 'message_pump;
                                    }
                                }

                                // Send EndIngest
                                let Ok((_, _, bytes)) = messages::WsMessage::EndIngest { unique_id: serial as u64, n_chunks: n_chunks as u64 }.to_bytes() else {
                                    error!("Failed to serialize EndIngest message");
                                    break 'message_pump;
                                };
                                if let Err(_) = socket_sender_tx.send(Message::Binary(bytes.into())).await {
                                    error!("Failed to send EndIngest message");
                                    break 'message_pump;
                                }
                                info!("Submitted {} bytes for ingestion", byte_count);
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
                                        info!("Control channel connected and permitted");
                                        set_license_status(LicenseStatus {
                                            license_type: license_state,
                                            trial_expires: expiration_date as i32,
                                        });
                                        permitted = true;
                                        sleep_seconds = 60;
                                    }
                                    messages::WsMessage::Heartbeat { timestamp } => {
                                        // Send a heartbeat reply
                                        info!("Received heartbeat, sending reply");
                                        let message = messages::WsMessage::HeartbeatReply { insight_time: timestamp };
                                        let Ok((_, _, bytes)) = message.to_bytes() else {
                                            error!("Failed to serialize heartbeat reply");
                                            break 'message_pump;
                                        };
                                        if let Err(_) = socket_sender_tx.send(Message::Binary(bytes.into())).await {
                                            error!("Failed to send heartbeat reply");
                                            break 'message_pump;
                                        }
                                    }
                                    messages::WsMessage::RemoteCommands { commands } => {
                                        crate::lts2_sys::lts2_client::enqueue(commands);
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
                                if let Err(_) = socket_sender_tx.send(Message::Pong(payload)).await {
                                    error!("Failed to send Pong message");
                                    break 'message_pump;
                                }
                            }
                            Message::Pong(..) => {
                                // Actual message - good
                            }
                            Message::Close(..) => {
                                info!("WebSocket closed by remote");
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
                        info!("Sending Ping message");
                        let bytes = vec![1u8; 4];
                        if let Err(_) = socket_sender_tx.send(Message::Ping(bytes.into())).await {
                            error!("Failed to send Ping message");
                            break 'message_pump;
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
                        if let Err(_) = socket_sender_tx.send(Message::Binary(bytes.into())).await {
                            error!("Failed to send license message");
                            break 'message_pump;
                        }
                    }
                }
            } // End of message pump
        }
        info!("Sleeping before reconnecting the persistent channel");
        tokio::time::sleep(Duration::from_secs(sleep_seconds)).await;
        sleep_seconds = 60;
    }
}

async fn connect() -> anyhow::Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
    let remote_host = crate::lts2_sys::lts2_client::get_remote_host();
    let target = format!("wss://{}:443/shaper_gateway/ws", remote_host);
    info!("Connecting to shaper gateway: {target}");
    
    let mut addresses = format!("{}:443", remote_host).to_socket_addrs()?;
    let Some(addr) = addresses.next() else {
        bail!("DNS Error");
    };
    
    // TCP Stream
    let Ok(Ok(stream)) = timeout(TCP_TIMEOUT, TcpStream::connect(&addr)).await else {
        warn!("Failed to connect to shaper gateway server: {remote_host}");
        bail!("Failed to connect to shaper gateway server".to_string());
    };

    // Native TLS
    info!("Connected to shaper gateway server: {remote_host}");
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
    info!("Connecting tungstenite client to shaper gateway server: {target}");
    let result =
        timeout(TCP_TIMEOUT, tokio_tungstenite::client_async_tls_with_config(target, stream, None, Some(t_connector)))
            .await;
    if result.is_err() {
        bail!("Failed to connect to shaper gateway server. {result:?}");
    }
    let Ok(Ok((socket, _response))) = result else {
        warn!("Failed to connect to shaper gateway server");
        bail!("Failed to connect to shaper gateway server. {result:?}");
    };
    info!("Connected");
    
    Ok(socket)
}

async fn send_magic_number(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> anyhow::Result<()> {
    const MAGIC_NUMBER: u32 = 0x8123a;
    let message = messages::WsMessage::Hello { magic: MAGIC_NUMBER };
    let (_raw_size, _compressed_size, bytes) = message.to_bytes()?;
    timeout(TCP_TIMEOUT, socket.send(tokio_tungstenite::tungstenite::Message::Binary(bytes.into()))).await??;
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
    timeout(TCP_TIMEOUT, socket.send(tokio_tungstenite::tungstenite::Message::Binary(bytes.into()))).await??;

    Ok(())
}
