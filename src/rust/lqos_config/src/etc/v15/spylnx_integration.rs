use allocative::Allocative;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct SplynxIntegration {
    pub enable_spylnx: bool,
    pub api_key: String,
    pub api_secret: String,
    pub url: String,
}

impl Default for SplynxIntegration {
    fn default() -> Self {
        SplynxIntegration {
            enable_spylnx: false,
            api_key: "".to_string(),
            api_secret: "".to_string(),
            url: "".to_string(),
        }
    }
}
