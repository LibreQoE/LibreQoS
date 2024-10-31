//! Anonymous statistics section of the configuration
//! file.

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct UsageStats {
    /// Are we allowed to send stats at all?
    pub send_anonymous: bool,

    /// Where do we send them?
    pub anonymous_server: String,
}

impl Default for UsageStats {
    fn default() -> Self {
        Self {
            send_anonymous: true,
            anonymous_server: "stats.libreqos.io:9125".to_string(),
        }
    }
}