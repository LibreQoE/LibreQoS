use super::{BusClientError, PREALLOCATE_CLIENT_BUFFER_BYTES};
use crate::{
  decode_response, encode_request, BusRequest, BusResponse, BusSession,
  BUS_SOCKET_PATH,
};
use log::{error, warn};
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
  pub async fn new() -> Result<Self, BusClientError> {
    Ok(Self {
      stream: Self::connect().await,
      buffer: vec![0u8; PREALLOCATE_CLIENT_BUFFER_BYTES],
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
  ) -> Result<Vec<BusResponse>, BusClientError> {
    if self.stream.is_none() {
      self.stream = Self::connect().await;
    }

    // If the stream isn't writeable, bail out
    if self.stream.is_some()
      && self.stream.as_ref().unwrap().writable().await.is_err()
    {
      // The stream has gone away
      self.stream = None;
      warn!("Local socket stream is no longer connected");
      return Err(BusClientError::StreamNotConnected);
    }

    // Encode the message
    let message = BusSession { persist: true, requests };
    let msg = encode_request(&message);
    if msg.is_err() {
      error!("Unable to encode request {:?}", message);
      return Err(BusClientError::EncodingError);
    }
    let msg = msg.unwrap();

    // Send with a timeout. If the timeout fails, then the stream went wrong
    if self.stream.is_some() {
      let timer =
        timeout(self.timeout, Self::send(self.stream.as_mut().unwrap(), &msg));
      let failed =
        if let Ok(inner) = timer.await { inner.is_err() } else { false };
      if failed {
        self.stream = None;
        warn!("Stream no longer connected");
        return Err(BusClientError::StreamNotConnected);
      }
    }

    // Receive with a timeout. If the timeout fails, then something went wrong.
    if self.stream.is_some() {
      let timer = timeout(
        self.timeout,
        self.stream.as_mut().unwrap().read(&mut self.buffer),
      );
      let failed =
        if let Ok(inner) = timer.await { inner.is_err() } else { false };
      if failed {
        warn!("Stream no longer connected");
        return Err(BusClientError::StreamNotConnected);
      }
    }

    let reply = decode_response(&self.buffer);
    if reply.is_err() {
      error!("Unable to decode response from socket.");
      return Err(BusClientError::DecodingError);
    }
    let reply = reply.unwrap();
    self.buffer.iter_mut().for_each(|b| *b = 0);

    Ok(reply.responses)
  }

  async fn send(
    stream: &mut UnixStream,
    msg: &[u8],
  ) -> Result<(), BusClientError> {
    let ret = stream.write(msg).await;
    if ret.is_err() {
      error!("Unable to write to {BUS_SOCKET_PATH} stream.");
      error!("{:?}", ret);
      return Err(BusClientError::StreamWriteError);
    }
    Ok(())
  }

  /// Returns `true` if the underlying socket is available
  /// This isn't perfect - the socket may die inbetween calling
  /// this function and trying to use it.
  pub fn is_connected(&self) -> bool {
    self.stream.is_some()
  }
}
