use crate::BusRequest;
use serde::{Deserialize, Serialize};

/// `BusSession` represents a complete session with `lqosd`. It must
/// contain a cookie value (defined in the `cookie_value()` function),
/// which serves as a sanity check that the connection is valid.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BusSession {
    /// Should the session stick around after this request?
    pub persist: bool,

    /// A list of requests to include in this session.
    pub requests: Vec<BusRequest>,
}
