use allocative::Allocative;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct NetzurIntegration {
    pub enable_netzur: bool,
    pub api_key: String,
    pub api_url: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub use_mikrotik_ipv6: bool,
}

fn default_timeout_secs() -> u64 {
    60
}

impl Default for NetzurIntegration {
    fn default() -> Self {
        NetzurIntegration {
            enable_netzur: false,
            api_key: "".to_string(),
            api_url: "".to_string(),
            timeout_secs: default_timeout_secs(),
            use_mikrotik_ipv6: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_timeout_is_sixty_seconds() {
        let integration = NetzurIntegration::default();
        assert_eq!(integration.timeout_secs, 60);
        assert!(!integration.enable_netzur);
        assert!(!integration.use_mikrotik_ipv6);
    }
}
