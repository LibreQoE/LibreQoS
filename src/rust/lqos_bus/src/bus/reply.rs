use serde::{Serialize, Deserialize};
use crate::BusResponse;

/// A single reply, always generated in response to a `BusSession` request.
/// Echoes the `auth_cookie` back to ensure that connectivity is valid,
/// and contains one or more `BusResponse` objects with the details
/// of the reply to each request.
/// 
/// No ordering guarantee is present. Responses may be out-of-order with
/// respect to the order of the requests.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BusReply {
    /// Auth cookie, which should match the output of the `auth_cookie`
    /// function.
    pub auth_cookie: u32,

    /// A list of `BusResponse` objects generated in response to the
    /// requests that started the session.
    pub responses: Vec<BusResponse>,
}