//! Tornado definitions (originally from ispConfig.py)

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct TornadoConfig {
    pub enabled: bool,
    pub targets: Vec<String>,
    pub dry_run: bool,
}

impl Default for TornadoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            targets: Vec::new(),
            dry_run: true,
        }
    }
}
