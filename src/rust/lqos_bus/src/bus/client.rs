use super::PREALLOCATE_CLIENT_BUFFER_BYTES;
use crate::{
  decode_response, encode_request, BusRequest, BusResponse, BusSession,
  BUS_SOCKET_PATH,
};
use anyhow::{Error, Result};
use tokio::{
  io::{AsyncReadExt, AsyncWriteExt},
  net::UnixStream,
};

/// Convenient wrapper for accessing the bus
///
/// ## Arguments
///
/// * `requests` a vector of `BusRequest` requests to make.
///
/// **Returns** Either an error, or a vector of `BusResponse` replies
pub async fn bus_request(
  requests: Vec<BusRequest>,
) -> Result<Vec<BusResponse>> {
  let stream = UnixStream::connect(BUS_SOCKET_PATH).await;
  match &stream {
    Err(e) => match e.kind() {
      std::io::ErrorKind::NotFound => {
        return Err(Error::msg(format!(
          "{} not found. Check permissions and that lqosd is running.",
          BUS_SOCKET_PATH
        )))
      }
      _ => {}
    },
    _ => {}
  }
  let mut stream = stream.unwrap();
  let test = BusSession { persist: false, requests };
  let msg = encode_request(&test)?;
  stream.write(&msg).await?;
  let mut buf = Vec::with_capacity(PREALLOCATE_CLIENT_BUFFER_BYTES);
  let _ = stream.read_to_end(&mut buf).await?;
  let reply = decode_response(&buf)?;

  Ok(reply.responses)
}
