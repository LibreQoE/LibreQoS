use allocative::Allocative;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct InfluxDbConfig {
    pub enable_influxdb: bool,
    pub url: String,
    pub bucket: String,
    pub org: String,
    pub token: String,
}

impl Default for InfluxDbConfig {
    fn default() -> Self {
        Self {
            enable_influxdb: false,
            url: "http://localhost:8086".to_string(),
            bucket: "libreqos".to_string(),
            org: "Your ISP Name".to_string(),
            token: "".to_string(),
        }
    }
}
