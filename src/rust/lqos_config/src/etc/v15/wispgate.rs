use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct WispGateIntegration {
    pub enable_wispgate: bool,
    pub wispgate_api_token: String,
    pub wispgate_api_url: String,
}