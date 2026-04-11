//! Dynamic circuit configuration.
//!
//! This module defines the configuration schema for the "dynamic circuits"
//! subsystem. Runtime behavior is implemented elsewhere; for now this is
//! configuration + validation only.

use allocative::Allocative;
use ip_network::IpNetwork;
use serde::{Deserialize, Serialize};

fn default_ttl_seconds() -> u64 {
    300
}

fn deserialize_ip_network_allow_default<'de, D>(deserializer: D) -> Result<IpNetwork, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    let trimmed = raw.trim();
    let normalized = match trimmed {
        "0.0.0.0" => "0.0.0.0/0",
        "::" => "::/0",
        other => other,
    };
    normalized
        .parse::<IpNetwork>()
        .map_err(serde::de::Error::custom)
}

fn serialize_ip_network<S>(value: &IpNetwork, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&value.to_string())
}

/// One rule that applies a default set of rates + optional attachment to an IP range.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct DynamicCircuitRangeRule {
    /// Human-readable rule name used in logs and diagnostics.
    pub name: String,
    /// IP network matched by this rule (CIDR notation).
    ///
    /// For convenience, `0.0.0.0` and `::` are accepted shorthands for the default
    /// routes (`0.0.0.0/0` and `::/0`).
    #[allocative(skip)]
    #[serde(
        serialize_with = "serialize_ip_network",
        deserialize_with = "deserialize_ip_network_allow_default"
    )]
    pub ip_range: IpNetwork,
    /// Minimum guaranteed downstream rate in Mbps.
    pub download_min_mbps: f32,
    /// Minimum guaranteed upstream rate in Mbps.
    pub upload_min_mbps: f32,
    /// Maximum downstream rate in Mbps.
    pub download_max_mbps: f32,
    /// Maximum upstream rate in Mbps.
    pub upload_max_mbps: f32,
    /// Optional attachment target in `network.json` (node name). Empty means no attachment.
    #[serde(default)]
    pub attach_to: String,
}

/// Dynamic circuits configuration.
///
/// This section is optional in the top-level config so older installations can
/// upgrade without needing to add a new config section.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct DynamicCircuitsConfig {
    /// Master switch for the dynamic circuits feature set.
    #[serde(default)]
    pub enabled: bool,
    /// Time-to-live in seconds for runtime dynamic circuit state.
    #[serde(default = "default_ttl_seconds")]
    pub ttl_seconds: u64,
    /// Enable promotion of unknown IPs into dynamic circuits (when a rule matches).
    #[serde(default)]
    pub enable_unknown_ip_promotion: bool,
    /// Rules evaluated against unknown IPs (and future dynamic overlay logic).
    #[serde(default)]
    pub ranges: Vec<DynamicCircuitRangeRule>,
}

impl Default for DynamicCircuitsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_seconds: default_ttl_seconds(),
            enable_unknown_ip_promotion: false,
            ranges: Vec::new(),
        }
    }
}

impl DynamicCircuitsConfig {
    /// Validates configured dynamic circuit settings.
    pub fn validate(&self) -> Result<(), String> {
        if self.ttl_seconds == 0 {
            return Err("dynamic_circuits.ttl_seconds must be > 0".to_string());
        }

        for (index, rule) in self.ranges.iter().enumerate() {
            let label = if rule.name.trim().is_empty() {
                format!("dynamic_circuits.ranges[{index}]")
            } else {
                format!("dynamic_circuits.ranges[{index}] ('{}')", rule.name.trim())
            };

            if rule.name.trim().is_empty() {
                return Err(format!("{label}: name must not be empty"));
            }

            if !rule.download_min_mbps.is_finite() || rule.download_min_mbps < 0.1 {
                return Err(format!("{label}: download_min_mbps must be >= 0.1"));
            }
            if !rule.upload_min_mbps.is_finite() || rule.upload_min_mbps < 0.1 {
                return Err(format!("{label}: upload_min_mbps must be >= 0.1"));
            }
            if !rule.download_max_mbps.is_finite() || rule.download_max_mbps < 0.2 {
                return Err(format!("{label}: download_max_mbps must be >= 0.2"));
            }
            if !rule.upload_max_mbps.is_finite() || rule.upload_max_mbps < 0.2 {
                return Err(format!("{label}: upload_max_mbps must be >= 0.2"));
            }
            if rule.download_min_mbps > rule.download_max_mbps {
                return Err(format!("{label}: download_min_mbps must be <= download_max_mbps"));
            }
            if rule.upload_min_mbps > rule.upload_max_mbps {
                return Err(format!("{label}: upload_min_mbps must be <= upload_max_mbps"));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_ipv4_shorthand() {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(deserialize_with = "deserialize_ip_network_allow_default")]
            ip_range: IpNetwork,
        }

        let parsed: Wrapper =
            toml::from_str(r#"ip_range = "0.0.0.0""#).expect("parse ip_range shorthand");
        assert_eq!(parsed.ip_range.to_string(), "0.0.0.0/0");
    }

    #[test]
    fn parses_default_ipv6_shorthand() {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(deserialize_with = "deserialize_ip_network_allow_default")]
            ip_range: IpNetwork,
        }

        let parsed: Wrapper =
            toml::from_str(r#"ip_range = "::""#).expect("parse ip_range shorthand");
        assert_eq!(parsed.ip_range.to_string(), "::/0");
    }
}
