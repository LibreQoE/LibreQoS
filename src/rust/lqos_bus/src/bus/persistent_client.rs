use anyhow::Result;
use tokio::{net::UnixStream, io::{AsyncWriteExt, AsyncReadExt}};
use crate::{BUS_SOCKET_PATH, BusRequest, BusResponse, BusSession, encode_request, decode_response};

pub struct BusClient {
    stream: UnixStream,
    buffer: Vec<u8>,
}

impl BusClient {
    pub async fn new() -> Result<Self> {
        let stream = UnixStream::connect(BUS_SOCKET_PATH).await?;
        Ok(Self {
            stream,
            buffer: vec![0u8; 10240],
        })
    }

    pub async fn request(&mut self, requests: Vec<BusRequest>) -> Result<Vec<BusResponse>> {
        let test = BusSession {
            persist: true,
            requests,
        };
        let msg = encode_request(&test)?;
        self.stream.write(&msg).await?;
        self.stream.read(&mut self.buffer).await.unwrap();
        let reply = decode_response(&self.buffer)?;
    
        Ok(reply.responses)
    }
}
