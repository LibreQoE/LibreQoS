use allocative::Allocative;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct SplynxIntegration {
    #[serde(alias = "enable_spylnx")]
    pub enable_splynx: bool,
    pub api_key: String,
    pub api_secret: String,
    pub url: String,
    #[serde(default = "default_strategy")]
    pub strategy: String,
}

fn default_strategy() -> String {
    "ap_only".to_string()
}

impl Default for SplynxIntegration {
    fn default() -> Self {
        SplynxIntegration {
            enable_splynx: false,
            api_key: "".to_string(),
            api_secret: "".to_string(),
            url: "".to_string(),
            strategy: "ap_only".to_string(),
        }
    }
}
