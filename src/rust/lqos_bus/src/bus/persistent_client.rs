use anyhow::Result;
use tokio::{net::UnixStream, io::{AsyncWriteExt, AsyncReadExt}};
use crate::{BUS_SOCKET_PATH, BusRequest, BusResponse, BusSession, encode_request, decode_response};

/// Provides a lqosd bus client that persists between connections. Useful for when you are
/// going to be repeatedly polling the bus for data (e.g. `lqtop`) and want to avoid the
/// overhead of an individual connection.
pub struct BusClient {
    stream: UnixStream,
    buffer: Vec<u8>,
}

impl BusClient {
    /// Instantiates a bus client, connecting to the bus stream and initializing
    /// a buffer.
    pub async fn new() -> Result<Self> {
        let stream = UnixStream::connect(BUS_SOCKET_PATH).await?;
        Ok(Self {
            stream,
            buffer: vec![0u8; 10240],
        })
    }

    /// Analagous to the singe-task `bus_request`, sends a request to the existing
    /// bus connection.
    pub async fn request(&mut self, requests: Vec<BusRequest>) -> Result<Vec<BusResponse>> {
        let test = BusSession {
            persist: true,
            requests,
        };
        let msg = encode_request(&test)?;
        self.stream.write(&msg).await?;
        self.stream.read(&mut self.buffer).await.unwrap();
        let reply = decode_response(&self.buffer)?;
        self.buffer.iter_mut().for_each(|b| *b=0);

        Ok(reply.responses)
    }
}
