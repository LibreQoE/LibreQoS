mod client;
mod persistent_client;
mod reply;
mod request;
mod response;
mod session;
mod unix_socket_server;
mod queue_data;
pub use client::bus_request;
use log::error;
pub use persistent_client::BusClient;
pub use reply::BusReply;
pub use request::{BusRequest, StatsRequest};
pub use response::BusResponse;
pub use session::BusSession;
use thiserror::Error;
pub use unix_socket_server::UnixSocketServer;
pub use queue_data::*;

/// The local socket path to which `lqosd` will bind itself,
/// listening for requets.
pub const BUS_SOCKET_PATH: &str = "/run/lqos/bus";

/// The directory containing the bus socket. Used for ensuring
/// that the directory exists.
pub(crate) const BUS_SOCKET_DIRECTORY: &str = "/run/lqos";

const PREALLOCATE_CLIENT_BUFFER_BYTES: usize = 10240;

/// Encodes a BusSession with `bincode`, providing a tight binary
/// representation of the request object for TCP transmission.
pub fn encode_request(
  request: &BusSession,
) -> Result<Vec<u8>, BusSerializationError> {
  match bincode::serialize(request) {
    Ok(data) => Ok(data),
    Err(e) => {
      error!("Unable to encode/serialize request.");
      error!("{:?}", e);
      Err(BusSerializationError::SerializationError)
    }
  }
}

/// Decodes bytes into a `BusSession`.
pub fn decode_request(
  bytes: &[u8],
) -> Result<BusSession, BusSerializationError> {
  match bincode::deserialize(bytes) {
    Ok(data) => Ok(data),
    Err(e) => {
      error!("Unable to decode/deserialize request");
      error!("{:?}", e);
      Err(BusSerializationError::DeserializationError)
    }
  }
}

/// Encodes a `BusReply` object with `bincode`.
pub fn encode_response(
  request: &BusReply,
) -> Result<Vec<u8>, BusSerializationError> {
  match bincode::serialize(request) {
    Ok(data) => Ok(data),
    Err(e) => {
      error!("Unable to encode/serialize request.");
      error!("{:?}", e);
      Err(BusSerializationError::SerializationError)
    }
  }
}

/// Decodes a `BusReply` object with `bincode`.
pub fn decode_response(
  bytes: &[u8],
) -> Result<BusReply, BusSerializationError> {
  match bincode::deserialize(bytes) {
    Ok(data) => Ok(data),
    Err(e) => {
      error!("Unable to decode/deserialize request");
      error!("{:?}", e);
      Err(BusSerializationError::DeserializationError)
    }
  }
}

#[derive(Error, Debug)]
pub enum BusSerializationError {
  #[error("Unable to serialize requested data into bincode format")]
  SerializationError,
  #[error("Unable to deserialize provided data into bincode format")]
  DeserializationError,
}

#[derive(Error, Debug)]
pub enum BusClientError {
  #[error("Socket (typically /run/lqos/bus) not found. Check that lqosd is running, and you have permission to access the socket path.")]
  SocketNotFound,
  #[error("Unable to encode request")]
  EncodingError,
  #[error("Unable to decode request")]
  DecodingError,
  #[error("Cannot write to socket")]
  StreamWriteError,
  #[error("Cannot read from socket")]
  StreamReadError,
  #[error("Stream is no longer connected")]
  StreamNotConnected,
}

#[cfg(test)]
mod test {
  use super::*;
  use crate::{BusRequest, BusResponse};

  #[test]
  fn test_session_roundtrip() {
    let session =
      BusSession { persist: false, requests: vec![BusRequest::Ping] };

    let bytes = encode_request(&session).unwrap();
    let new_session = decode_request(&bytes).unwrap();
    assert_eq!(new_session.requests.len(), session.requests.len());
    assert_eq!(new_session.requests[0], session.requests[0]);
  }

  #[test]
  fn test_reply_roundtrip() {
    let reply = BusReply { responses: vec![BusResponse::Ack] };
    let bytes = encode_response(&reply).unwrap();
    let new_reply = decode_response(&bytes).unwrap();
    assert_eq!(reply.responses.len(), new_reply.responses.len());
    assert_eq!(reply.responses[0], new_reply.responses[0]);
  }
}
