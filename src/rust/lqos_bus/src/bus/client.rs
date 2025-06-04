use crate::{
    bus::{BusClientError}, BusReply, BusRequest, BusResponse, BusSession, BUS_SOCKET_PATH
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};
use tracing::error;

pub(crate) const MAGIC_NUMBER: [u8; 4] = [0x4C, 0x52, 0x45, 0x51]; // "LREQ"
pub(crate) const MAGIC_RESPONSE: [u8; 4] = [0x4C, 0x52, 0x45, 0x50]; // "LREP"

/// A client for the libreqos bus, which connects to the bus socket and sends requests.
/// The client is persistent by default, disconnecting when dropped.
pub struct LibreqosBusClient {
    stream: UnixStream,
    request_id: u64,
}

impl LibreqosBusClient {
    /// Creates a new `LibreqosBusClient`.
    pub async fn new() -> Result<Self, BusClientError> {
        let Ok(mut stream) = UnixStream::connect(BUS_SOCKET_PATH).await else {
            return Err(BusClientError::SocketNotFound);
        };

        // Send the magic number to the bus
        stream.write(&MAGIC_NUMBER).await.map_err(|_| {
            error!("Unable to write magic number to {BUS_SOCKET_PATH} stream.");
            BusClientError::StreamWriteError
        })?;

        // Read the response magic number
        let mut buf = [0u8; 4];
        stream.read_exact(&mut buf).await.map_err(|_| {
            error!("Unable to read magic number from {BUS_SOCKET_PATH} stream.");
            BusClientError::StreamReadError
        })?;
        if buf != MAGIC_RESPONSE {
            error!("Received invalid magic number from {BUS_SOCKET_PATH} stream.");
            return Err(BusClientError::StreamReadError);
        }

        Ok(Self {
            stream,
            request_id: 0,
        })
    }

    /// Sends a request to the bus and waits for a response.
    ///
    /// ## Arguments
    /// * `requests` a vector of `BusRequest` requests to make.
    /// 
    /// **Returns** Either an error, or a vector of `BusResponse` replies
    pub async fn request(&mut self, requests: Vec<BusRequest>) -> Result<Vec<BusResponse>, BusClientError> {
        let request_id = self.request_id;
        self.request_id += 1;
        // Mirror the code in unix_socket_server::listen

        let session = BusSession {
            requests,
        };
        let Ok(session_bytes) = bincode::serialize(&session) else {
            error!("Unable to serialize session.");
            return Err(BusClientError::EncodingError);
        };
        let size = session_bytes.len();
        self.stream.write_u64_le(request_id).await.map_err(|_| {
            error!("Unable to write request ID to {BUS_SOCKET_PATH} stream.");
            BusClientError::StreamWriteError
        })?;
        self.stream.write_u64_le(size as u64).await.map_err(|_| {
            error!("Unable to write session size to {BUS_SOCKET_PATH} stream.");
            BusClientError::StreamWriteError
        })?;
        self.stream.write_all(&session_bytes).await.map_err(|_| {
            error!("Unable to write session to {BUS_SOCKET_PATH} stream.");
            BusClientError::StreamWriteError
        })?;

        // Read the response
        let mut response_id = [0u8; 8];
        self.stream.read_exact(&mut response_id).await.map_err(|_| {
            error!("Unable to read response ID from {BUS_SOCKET_PATH} stream.");
            BusClientError::StreamReadError
        })?;
        let response_id = u64::from_le_bytes(response_id);
        if response_id != request_id {
            error!("Received response ID {response_id} does not match request ID {request_id}.");
            return Err(BusClientError::StreamReadError);
        }
        let mut response_size = [0u8; 8];
        self.stream.read_exact(&mut response_size).await.map_err(|_| {
            error!("Unable to read response size from {BUS_SOCKET_PATH} stream.");
            BusClientError::StreamReadError
        })?;
        let response_size = u64::from_le_bytes(response_size) as usize;
        if response_size == 0 {
            return Ok(Vec::new());
        }
        let mut response_bytes = vec![0u8; response_size];
        self.stream.read_exact(&mut response_bytes).await.map_err(|_| {
            error!("Unable to read response from {BUS_SOCKET_PATH} stream.");
            BusClientError::StreamReadError
        })?;
        let response: BusReply = match bincode::deserialize(&response_bytes) {
            Ok(response) => response,
            Err(e) => {
                error!("Unable to deserialize response: {:?}", e);
                return Err(BusClientError::DecodingError);
            }
        };
        if response.responses.is_empty() {
            return Ok(Vec::new());
        }
        if response.responses.len() != session.requests.len() {
            error!("Received {} responses, expected {}", response.responses.len(), session.requests.len());
            return Err(BusClientError::DecodingError);
        }
        Ok(response.responses)
    }
}

/// Convenient wrapper for accessing the bus, for a single request-response cycle. This
/// is NOT the most efficient way to access the bus: a persistent client will perform better
/// when there are multiple requests to be made.
///
/// ## Arguments
///
/// * `requests` a vector of `BusRequest` requests to make.
///
/// **Returns** Either an error, or a vector of `BusResponse` replies
pub async fn bus_request(requests: Vec<BusRequest>) -> Result<Vec<BusResponse>, BusClientError> {
    let Ok(mut client) = LibreqosBusClient::new().await else {
        return Err(BusClientError::SocketNotFound);
    };
    client.request(requests).await
}
