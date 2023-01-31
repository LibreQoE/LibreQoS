use super::PREALLOCATE_CLIENT_BUFFER_BYTES;
use crate::{
  decode_response, encode_request, BusRequest, BusResponse, BusSession,
  BUS_SOCKET_PATH, bus::BusClientError,
};
use tokio::{
  io::{AsyncReadExt, AsyncWriteExt},
  net::UnixStream,
};
use log::error;

/// Convenient wrapper for accessing the bus
///
/// ## Arguments
///
/// * `requests` a vector of `BusRequest` requests to make.
///
/// **Returns** Either an error, or a vector of `BusResponse` replies
pub async fn bus_request(
  requests: Vec<BusRequest>,
) -> Result<Vec<BusResponse>, BusClientError> {
  let stream = UnixStream::connect(BUS_SOCKET_PATH).await;
  match &stream {
    Err(e) => match e.kind() {
      std::io::ErrorKind::NotFound => {
        error!("Unable to access {BUS_SOCKET_PATH}. Check that lqosd is running and you have appropriate permissions.");
        return Err(BusClientError::SocketNotFound);
      }
      _ => {}
    },
    _ => {}
  }
  let mut stream = stream.unwrap(); // This unwrap is safe, we checked that it exists previously
  let test = BusSession { persist: false, requests };
  let msg = encode_request(&test);
  if msg.is_err() {
    error!("Unable to encode request {:?}", test);
    return Err(BusClientError::EncodingError);
  }
  let msg = msg.unwrap();
  let ret = stream.write(&msg).await;
  if ret.is_err() {
    error!("Unable to write to {BUS_SOCKET_PATH} stream.");
    error!("{:?}", ret);
    return Err(BusClientError::StreamWriteError);
  }
  let mut buf = Vec::with_capacity(PREALLOCATE_CLIENT_BUFFER_BYTES);
  let ret = stream.read_to_end(&mut buf).await;
  if ret.is_err() {
    error!("Unable to read from {BUS_SOCKET_PATH} stream.");
    error!("{:?}", ret);
    return Err(BusClientError::StreamReadError);
  }
  let reply = decode_response(&buf);
  if reply.is_err() {
    error!("Unable to decode response from socket.");
    return Err(BusClientError::DecodingError);
  }
  let reply = reply.unwrap();
  Ok(reply.responses)
}
