use crate::{
  decode_response, encode_request, BusRequest, BusResponse, BusSession,
  BUS_SOCKET_PATH,
};
use anyhow::{Error, Result};
use std::time::Duration;
use tokio::{
  io::{AsyncReadExt, AsyncWriteExt},
  net::UnixStream,
  time::timeout,
};

/// Provides a lqosd bus client that persists between connections. Useful for when you are
/// going to be repeatedly polling the bus for data (e.g. `lqtop`) and want to avoid the
/// overhead of an individual connection.
pub struct BusClient {
  stream: Option<UnixStream>,
  buffer: Vec<u8>,
  timeout: Duration,
}

impl BusClient {
  /// Instantiates a bus client, connecting to the bus stream and initializing
  /// a buffer.
  pub async fn new() -> Result<Self> {
    Ok(Self {
      stream: Self::connect().await,
      buffer: vec![0u8; 10240],
      timeout: Duration::from_millis(100),
    })
  }

  async fn connect() -> Option<UnixStream> {
    if let Ok(stream) = UnixStream::connect(BUS_SOCKET_PATH).await {
      Some(stream)
    } else {
      None
    }
  }

  /// Analagous to the singe-task `bus_request`, sends a request to the existing
  /// bus connection.
  pub async fn request(
    &mut self,
    requests: Vec<BusRequest>,
  ) -> Result<Vec<BusResponse>> {
    if self.stream.is_none() {
      self.stream = Self::connect().await;
    }

    // If the stream isn't writeable, bail out
    if self.stream.is_some()
      && self.stream.as_ref().unwrap().writable().await.is_err()
    {
      // The stream has gone away
      self.stream = None;
      return Err(Error::msg("Stream not connected"));
    }

    // Encode the message
    let message = BusSession { persist: true, requests };
    let msg = encode_request(&message)?;

    // Send with a timeout. If the timeout fails, then the stream went wrong
    if self.stream.is_some() {
      let timer = timeout(
        self.timeout,
        Self::send(self.stream.as_mut().unwrap(), &msg),
      );
      let failed = if let Ok(inner) = timer.await {
        if inner.is_err() {
          true
        } else {
          false
        }
      } else {
        false
      };
      if failed {
        self.stream = None;
        return Err(Error::msg("Stream not connected"));
      }
    }

    // Receive with a timeout. If the timeout fails, then something went wrong.
    if self.stream.is_some() {
      let timer = timeout(
        self.timeout,
        self.stream.as_mut().unwrap().read(&mut self.buffer),
      );
      let failed = if let Ok(inner) = timer.await {
        if inner.is_err() {
          true
        } else {
          false
        }
      } else {
        false
      };
      if failed {
        self.stream = None;
        return Err(Error::msg("Stream not connected"));
      }
    }

    let reply = decode_response(&self.buffer)?;
    self.buffer.iter_mut().for_each(|b| *b = 0);

    Ok(reply.responses)
  }

  async fn send(stream: &mut UnixStream, msg: &[u8]) -> Result<()> {
    stream.write(&msg).await?;
    Ok(())
  }

  pub fn is_connected(&self) -> bool {
    self.stream.is_some()
  }
}
