use crate::BusRequest;
use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// `BusSession` represents a complete session with `lqosd`. It must
/// contain a cookie value (defined in the `cookie_value()` function),
/// which serves as a sanity check that the connection is valid.
#[derive(Serialize, Deserialize, Clone, Debug, Allocative)]
pub struct BusSession {
    /// A list of requests to include in this session.
    pub requests: Vec<BusRequest>,
}
