use crate::BusRequest;
use serde::{Deserialize, Serialize};

/// `BusSession` represents a complete session with `lqosd`. It must
/// contain a cookie value (defined in the `cookie_value()` function),
/// which serves as a sanity check that the connection is valid.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BusSession {
    /// Authentication cookie that must match the `auth_cookie()` function's
    /// return value.
    pub auth_cookie: u32,

    /// A list of requests to include in this session.
    pub requests: Vec<BusRequest>,
}
