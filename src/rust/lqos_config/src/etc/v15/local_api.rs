//! Configuration for the local LibreQoS API service.

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Maximum number of named local API keys stored in the configuration.
pub const MAX_LOCAL_API_KEYS: usize = 16;

/// Non-secret metadata and the digest for one named local API key.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct LocalApiKeyConfig {
    /// Stable UUID included in the generated key.
    pub id: String,
    /// Administrator-provided display name.
    pub name: String,
    /// Lowercase hexadecimal SHA-256 digest of the complete generated key.
    pub token_sha256: String,
    /// Unix timestamp at which the key was created.
    pub created_at_unix: u64,
}

/// Local API authentication settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Allocative)]
pub struct LocalApiConfig {
    /// Optional legacy bearer token accepted by the local API.
    ///
    /// This token authenticates callers but does not bypass mapped-circuit
    /// licensing limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_token: Option<String>,

    /// Named local API keys. Only digests, never raw generated keys, are stored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<LocalApiKeyConfig>,
}

#[cfg(test)]
mod tests {
    use super::LocalApiConfig;

    #[test]
    fn defaults_support_configs_without_named_keys() {
        let config: LocalApiConfig = toml::from_str("").expect("empty section should load");
        assert_eq!(config, LocalApiConfig::default());
    }

    #[test]
    fn named_keys_round_trip_with_legacy_token() {
        let raw = r#"
bearer_token = "legacy-token"

[[keys]]
id = "00000000-0000-0000-0000-000000000001"
name = "Monitoring"
token_sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
created_at_unix = 42
"#;
        let config: LocalApiConfig = toml::from_str(raw).expect("named key config should load");
        assert_eq!(config.bearer_token.as_deref(), Some("legacy-token"));
        assert_eq!(config.keys.len(), 1);
        let encoded = toml::to_string(&config).expect("config should serialize");
        let decoded: LocalApiConfig = toml::from_str(&encoded).expect("config should round trip");
        assert_eq!(decoded, config);
    }
}
