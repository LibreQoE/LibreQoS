mod session;
mod request;
mod reply;
mod response;
pub use session::BusSession;
pub use request::BusRequest;
pub use reply::BusReply;
pub use response::BusResponse;
use anyhow::Result;

/// The address to which `lqosd` should bind itself when listening for
/// local bust requests.
/// 
/// This is typically `localhost` to minimize the exposed footprint.
pub const BUS_BIND_ADDRESS: &str = "127.0.0.1:9999";

/// Encodes a BusSession with `bincode`, providing a tight binary
/// representation of the request object for TCP transmission.
pub fn encode_request(request: &BusSession) -> Result<Vec<u8>> {
    Ok(bincode::serialize(request)?)
}

/// Decodes bytes into a `BusSession`.
pub fn decode_request(bytes: &[u8]) -> Result<BusSession> {
    Ok(bincode::deserialize(&bytes)?)
}

/// Encodes a `BusReply` object with `bincode`.
pub fn encode_response(request: &BusReply) -> Result<Vec<u8>> {
    Ok(bincode::serialize(request)?)
}

/// Decodes a `BusReply` object with `bincode`.
pub fn decode_response(bytes: &[u8]) -> Result<BusReply> {
    Ok(bincode::deserialize(&bytes)?)
}

/// The cookie value to use to determine that the session is valid.
pub fn cookie_value() -> u32 {
    1234
}