use crate::node_manager::shaper_queries_actor::caches::Caches;
use crate::node_manager::shaper_queries_actor::ws_message::WsMessage;
use anyhow::{anyhow, bail};
use futures_util::stream::{SplitSink, StreamExt};
use futures_util::SinkExt;
use lqos_config::load_config;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, info, warn};

const TCP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub enum RemoteInsightCommand {
    Ping,
    ShaperThroughput { seconds: i32 },
    ShaperPackets { seconds: i32 },
    ShaperPercent { seconds: i32 },
    ShaperFlows { seconds: i32 },
    ShaperRttHistogram { seconds: i32 },
}

pub struct RemoteInsight {
    tx: Mutex<Option<tokio::sync::mpsc::Sender<RemoteInsightCommand>>>,
    caches: Arc<Caches>,
}

impl RemoteInsight {
    pub fn new(caches: Arc<Caches>) -> Self {
        Self { tx: Mutex::new(None), caches }
    }

    async fn connect(&self) {
        let (tx, rx) = tokio::sync::mpsc::channel(128);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(run_remote_insight(rx, ready_tx, self.caches.clone()));
        let _ = ready_rx.await;
        let mut lock = self.tx.lock().await;
        *lock = Some(tx);
        drop(lock);
        info!("Connected to remote insight (processor layer)");
    }

    pub async fn command(&self, command: RemoteInsightCommand) -> bool
    {
        info!("RICMD - Sending command: {command:?}");
        let lock_exists = self.tx.lock().await.is_some();
        if !lock_exists {
            self.connect().await;
        }

        let mut failed = false;
        let mut tx = self.tx.lock().await;
        if let Some(tx) = tx.as_ref() {
            let ping = tx.send(RemoteInsightCommand::Ping).await;
            if ping.is_err() {
                failed = true;
            }

            let result = tx.send(command.clone()).await;
            if result.is_err() {
                failed = true;
            }
        }
        if failed {
            *tx = None;
        }
        info!("RICMD - Sent command: {command:?}, Failed? {failed}");
        failed
    }
}

async fn run_remote_insight(
    mut command: tokio::sync::mpsc::Receiver<RemoteInsightCommand>,
    ready: tokio::sync::oneshot::Sender<()>,
    caches: Arc<Caches>,
) -> anyhow::Result<()> {
    let mut socket = connect().await?;
    let (mut write, mut read) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel(128);

    // Negotiation
    debug!("Waiting for IdentifyYourself");
    let msg = read.next().await;
    let Some(Ok(msg)) = msg else {
        warn!("Failed to read from shaper query server");
        bail!("Failed to read from shaper query server");
    };
    let tungstenite::Message::Binary(payload) = msg else {
        warn!("Failed to read from shaper query server");
        bail!("Failed to read from shaper query server");
    };
    let message = WsMessage::from_bytes(&payload)?;
    match message {
        WsMessage::IdentifyYourself => {
            debug!("Sending Hello");
            send_hello(&mut write).await?;
        }
        _ => {
            warn!("Unexpected message from shaper query server");
            bail!("Unexpected message from shaper query server");
        }
    }

    // Wait for a TokenInvalid or TokenValid
    debug!("Waiting for token response");
    let msg = read.next().await;
    let Some(Ok(msg)) = msg else {
        warn!("Failed to read from shaper query server");
        bail!("Failed to read from shaper query server");
    };
    let tungstenite::Message::Binary(payload) = msg else {
        warn!("Failed to read from shaper query server");
        bail!("Failed to read from shaper query server");
    };
    let message = WsMessage::from_bytes(&payload)?;
    match message {
        WsMessage::TokenAccepted => {
            info!("Token accepted");
        }
        WsMessage::InvalidToken => {
            warn!("Invalid token");
            bail!("Invalid token");
        }
        _ => {
            warn!("Unexpected message from shaper query server");
            bail!("Unexpected message from shaper query server");
        }
    }

    // Ready
    debug!("Ready to receive commands");
    ready.send(()).map_err(|_| anyhow!("Failed to send ready message"))?;

    let timeout = Duration::from_secs(10);
    let mut ticker = tokio::time::interval(timeout);
    let mut last_action = tokio::time::Instant::now();
    loop {
        select! {
            _ = ticker.tick() => {
                if last_action.elapsed().as_secs_f32() > 59.9 {
                    info!("RI Shaper WSS timeout reached");
                    tx.send(tungstenite::Message::Close(None)).await?;
                }
            }
            command = command.recv() => {
                info!("RI Received command: {command:?}");
                last_action = tokio::time::Instant::now();
                match command {
                    None => tx.send(tungstenite::Message::Close(None)).await?,
                    Some(RemoteInsightCommand::Ping) => {
                        // Do nothing - this ensures the channel is alive
                    }
                    Some(RemoteInsightCommand::ShaperThroughput { seconds }) => {
                        let msg = WsMessage::ShaperThroughput { seconds }.to_bytes()?;
                        tx.send(tungstenite::Message::Binary(msg)).await?;
                    }
                    Some(RemoteInsightCommand::ShaperPackets { seconds }) => {
                        let msg = WsMessage::ShaperPackets { seconds }.to_bytes()?;
                        tx.send(tungstenite::Message::Binary(msg)).await?;
                    }
                    Some(RemoteInsightCommand::ShaperPercent { seconds }) => {
                        let msg = WsMessage::ShaperPercent { seconds }.to_bytes()?;
                        tx.send(tungstenite::Message::Binary(msg)).await?;
                    }
                    Some(RemoteInsightCommand::ShaperFlows { seconds }) => {
                        let msg = WsMessage::ShaperFlows { seconds }.to_bytes()?;
                        tx.send(tungstenite::Message::Binary(msg)).await?;
                    }
                    Some(RemoteInsightCommand::ShaperRttHistogram { seconds }) => {
                        let msg = WsMessage::ShaperRttHistogram { seconds }.to_bytes()?;
                        tx.send(tungstenite::Message::Binary(msg)).await?;
                    }
                }
            }
            msg = read.next() => {
                info!("RI Received message.");
                last_action = tokio::time::Instant::now();
                let Some(Ok(msg)) = msg else {
                    warn!("Failed to read from shaper query server");
                    break;
                };
                match msg {
                    tungstenite::Message::Ping(_) => {
                        write.send(tokio_tungstenite::tungstenite::Message::Pong(vec![])).await?;
                    }
                    tungstenite::Message::Pong(_) => {
                        // Ignore
                    }
                    tungstenite::Message::Close(_) => {
                        info!("Shaper query server closed the connection");
                        break;
                    }
                    tungstenite::Message::Frame(_) => {
                        warn!("Received a frame message from shaper query server");
                    }
                    tungstenite::Message::Text(_) => {
                        warn!("Received a text message from shaper query server");
                    }
                    tungstenite::Message::Binary(bytes) => {
                        let message = WsMessage::from_bytes(&bytes)?;
                        match message {
                            WsMessage::IdentifyYourself => {
                                warn!("Unexpected IdentifyYourself")
                                //send_hello(tx.clone()).await?;
                            }
                            WsMessage::TokenAccepted => {
                                info!("Token accepted");
                            }
                            WsMessage::InvalidToken => {
                                warn!("Invalid token");
                                tx.send(tungstenite::Message::Close(None)).await?
                            }
                            WsMessage::Tick => {
                                info!("Tick");
                            }
                            WsMessage::QueryResult {tag, seconds, data} => {
                                info!("Query result: {tag} {seconds}, length: {}", data.len());
                                caches.store(tag, seconds, data).await;
                            }
                            _ => unimplemented!()
                        }
                    }
                }
            }
            Some(to_send) = rx.recv() => {
                info!("RI Sending message.");
                last_action = tokio::time::Instant::now();
                write.send(to_send).await?;
            }
        }
    }
    warn!("RI Closing");
    Ok(())
}

async fn connect() -> anyhow::Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
    let remote_host = crate::lts2_sys::lts2_client::get_remote_host();
    let target = format!("wss://{}:443/shaper_api/shaperWs", remote_host);
    debug!("Connecting to shaper query server: {target}");
    let mut addresses = format!("{}:443", remote_host).to_socket_addrs()?;
    let addr = addresses.next().ok_or_else(|| anyhow!("Failed to resolve remote host"))?;

    // TCP Stream
    let Ok(Ok(stream)) = timeout(TCP_TIMEOUT, TcpStream::connect(&addr)).await else {
        warn!("Failed to connect to shaper query server: {remote_host}");
        bail!("Failed to connect to shaper query server");
    };

    // Native TLS
    debug!("Connected to shaper query server: {remote_host}");
    let Ok(connector) = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build() else
    {
        warn!("Failed to create TLS connector");
        bail!("Failed to create TLS connector");
    };
    let t_connector = tokio_tungstenite::Connector::NativeTls(connector);

    // Tungstenite Client
    debug!("Connecting tungstenite client to shaper query server: {target}");
    let result = tokio_tungstenite::client_async_tls_with_config(target, stream, None, Some(t_connector)).await;
    if result.is_err() {
        bail!("Failed to connect to shaper query server. {result:?}");
    }
    if result.is_err() {
        bail!("Failed to connect to shaper query server. {result:?}");
    }
    let Ok((socket, _response)) = result else {
        warn!("Failed to connect to shaper query server");
        bail!("Failed to connect to shaper query server");
    };
    debug!("Connected");
    Ok(socket)
}

async fn send_hello(write: &mut SplitSink<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, tungstenite::Message>) -> anyhow::Result<()> {
    let config = load_config()?;
    let Some(license_key) = &config.long_term_stats.license_key else {
        warn!("No license key found in config");
        bail!("No license key found in config");
    };

    let msg = WsMessage::Hello {
        license_key: license_key.to_string(),
        node_id: config.node_id.to_string(),
    }.to_bytes()?;
    //tx.send(tungstenite::Message::Binary(msg)).await?;
    write.send(tungstenite::Message::Binary(msg)).await?;

    Ok(())
}
