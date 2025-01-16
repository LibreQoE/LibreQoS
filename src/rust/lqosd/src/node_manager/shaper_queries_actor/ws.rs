use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;
use anyhow::{anyhow, bail};
use native_tls::TlsStream;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use tungstenite::{Message, WebSocket};
use lqos_config::load_config;
use crate::node_manager::shaper_queries_actor::Caches;

pub fn get_remote_data(caches: &mut Caches, seconds: i32) -> anyhow::Result<()> {
    info!("Getting remote data for {seconds} seconds");
    let config = load_config()?;
    let Some(license_key) = &config.long_term_stats.license_key else {
        warn!("No license key found in config");
        bail!("No license key found in config");
    };

    let mut socket = connect_shaper_socket()?;
    info!("Saying hello");
    send_hello(&mut socket, license_key.as_str(), &config.node_id)?;
    info!("Authorized");

    request_graphs(&mut socket, seconds)?;

    // TODO: Loop on all requests here
    let response = socket.read()?;
    let reply = WsMessage::from_bytes(&response.into_data())?;
    let WsMessage::QueryResult { tag, data } = reply else {
        bail!("Failed to get data from shaper query server");
    };
    match tag.as_str() {
        "throughput" => {
            let throughput = serde_cbor::from_slice(&data)?;
            caches.throughput.insert(seconds, throughput);
        }
        _ => {
            warn!("Unknown tag received from shaper query server: {tag}");
        }
    }

    Ok(())
}

#[derive(Serialize, Deserialize, Debug)]
enum WsMessage {
    // Requests
    IdentifyYourself,
    InvalidToken,
    TokenAccepted,
    ShaperThroughput { seconds: i32 },

    // Responses
    Hello { license_key: String, node_id: String },
    QueryResult { tag: String, data: Vec<u8> },
    Tick,
}

type Wss = WebSocket<TlsStream<TcpStream>>;

fn connect_shaper_socket() -> anyhow::Result<Wss> {
    let remote_host = crate::lts2_sys::lts2_client::get_remote_host();
    let target = format!("wss://{}:443/shaper_api/shaperWs", remote_host);
    info!("Connecting to shaper query server: {target}");
    let addresses = format!("{}:443", remote_host);
    let mut addresses = format!("{}:443", remote_host).to_socket_addrs()?;
    let addr = addresses.next().ok_or_else(|| anyhow!("Failed to resolve remote host"))?;

    let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(10)) else {
        warn!("Failed to connect to shaper query server: {remote_host}");
        bail!("Failed to connect to shaper query server");
    };
    info!("Connected to shaper query server: {remote_host}");
    let Ok(connector) = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build() else
    {
        warn!("Failed to create TLS connector");
        bail!("Failed to create TLS connector");
    };
    info!("Connecting TLS stream to shaper query server: {remote_host}");
    let result = connector.connect(&format!("{}", remote_host), stream);
    if result.is_err() {
        warn!("Failed to connect TLS stream to shaper query server: {result:?}");
        bail!("Failed to connect TLS stream to shaper query server: {result:?}");
    }
    let Ok(tls_stream) = result else {
        warn!("Failed to connect TLS stream to shaper query server");
        bail!("Failed to connect TLS stream to shaper query server");
    };
    info!("Connecting tungstenite client to shaper query server: {target}");
    let result = tungstenite::client(target, tls_stream);
    if result.is_err() {
        bail!("Failed to connect to shaper query server. {result:?}");
    }
    let Ok((mut socket, _response)) = result else {
        warn!("Failed to connect to shaper query server");
        bail!("Failed to connect to shaper query server");
    };
    info!("Connected");

    let Ok(msg) = socket.read() else {
        warn!("Failed to read from shaper query server");
        bail!("Failed to read from shaper query server");
    };
    let reply = WsMessage::from_bytes(&msg.into_data())?;
    let WsMessage::IdentifyYourself = reply else {
        warn!("Failed to identify with shaper query server. Got: {reply:?}");
        bail!("Failed to identify with shaper query server");
    };
    Ok(socket)
}

fn send_hello(socket: &mut Wss, license_key: &str, node_id: &str) -> anyhow::Result<()> {
    let msg = WsMessage::Hello {
        license_key: license_key.to_string(),
        node_id: node_id.to_string(),
    }.to_bytes()?;
    socket.send(Message::Binary(msg))?;
    let response = socket.read()?;
    let reply = WsMessage::from_bytes(&response.into_data())?;
    let WsMessage::TokenAccepted = reply else {
        warn!("Failed to authenticate with shaper query server. Got: {reply:?}");
        bail!("Failed to authenticate with shaper query server");
    };
    Ok(())
}

fn close(socket: &mut Wss) -> anyhow::Result<()> {
    // Close the socket
    socket.send(Message::Close(None))?;
    Ok(())
}

fn request_graphs(socket: &mut Wss, seconds: i32) -> anyhow::Result<()> {
    info!("Requesting throughput for {seconds} seconds");
    let msg = WsMessage::ShaperThroughput { seconds }.to_bytes()?;
    socket.send(Message::Binary(msg))?;
    Ok(())
}

impl WsMessage {
    fn to_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let raw_bytes = serde_cbor::to_vec(self)?;
        Ok(raw_bytes)
    }

    fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        Ok(serde_cbor::from_slice(&bytes)?)
    }
}