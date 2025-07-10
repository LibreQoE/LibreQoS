use allocative::Allocative;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct PowercodeIntegration {
    pub enable_powercode: bool,
    pub powercode_api_key: String,
    pub powercode_api_url: String,
}

impl Default for PowercodeIntegration {
    fn default() -> Self {
        PowercodeIntegration {
            enable_powercode: false,
            powercode_api_key: "".to_string(),
            powercode_api_url: "".to_string(),
        }
    }
}
