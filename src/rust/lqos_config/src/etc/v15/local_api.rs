//! Configuration for the local LibreQoS API service.

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Local API authentication settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Allocative)]
pub struct LocalApiConfig {
    /// Optional bearer token accepted by the local API.
    ///
    /// This token authenticates callers but does not bypass mapped-circuit
    /// licensing limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_token: Option<String>,
}
