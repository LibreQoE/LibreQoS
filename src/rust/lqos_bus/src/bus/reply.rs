use crate::BusResponse;
use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// A single reply, always generated in response to a `BusSession` request.
/// Echoes the `auth_cookie` back to ensure that connectivity is valid,
/// and contains one or more `BusResponse` objects with the details
/// of the reply to each request.
///
/// No ordering guarantee is present. Responses may be out-of-order with
/// respect to the order of the requests.
#[derive(Serialize, Deserialize, Clone, Debug, Allocative)]
pub struct BusReply {
    /// A list of `BusResponse` objects generated in response to the
    /// requests that started the session.
    pub responses: Vec<BusResponse>,
}
