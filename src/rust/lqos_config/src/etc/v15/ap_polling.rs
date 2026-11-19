//! `[ap_polling]` control-plane configuration.
//!
//! The section is optional. A configuration without it is exactly the
//! default surface: disabled, placement rewrites disabled, interval 300 s,
//! v2c timeout 1500 ms, three failures before an alert, capacity trust
//! factor 1.0, and empty credentials (the empty community falls back to the
//! existing global `[sonar_integration].snmp_community`).
//!
//! This module only parses and validates the surface. It never starts a
//! poller and never mutates any other setting.

use allocative::Allocative;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Default AP polling interval (seconds).
pub const DEFAULT_AP_POLLING_INTERVAL_S: u64 = 300;
/// Minimum accepted AP polling interval (seconds).
pub const MIN_AP_POLLING_INTERVAL_S: u64 = 30;
/// Default per-device SNMP timeout (milliseconds).
pub const DEFAULT_AP_POLLING_SNMP_TIMEOUT_MS: u64 = 1500;
/// Default consecutive failures before an alert.
pub const DEFAULT_AP_POLLING_FAILURES_BEFORE_ALERT: u32 = 3;
/// Default per-model capacity trust factor: 1.0, used as-is.
pub const DEFAULT_AP_POLLING_CAPACITY_FACTOR: f64 = 1.0;

fn default_v3_auth() -> String {
    "sha256".to_string()
}

fn default_v3_priv() -> String {
    "aes".to_string()
}

/// AP polling configuration (`[ap_polling]`).
///
/// Unknown keys are tolerated: serde ignores them and the section still
/// parses. Every field has a default, so a partial section is valid.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
#[serde(default)]
pub struct ApPollingConfig {
    /// Whether AP polling is enabled. Defaults to `false`.
    pub enabled: bool,
    /// Whether the supplement may rewrite circuit parents from polled
    /// observations. Defaults to `false`: v1 is capacity-only and the CRM
    /// `access_device` pin stays authoritative for placement.
    pub apply_placement: bool,
    /// Poll interval in seconds. Must be at least
    /// [`MIN_AP_POLLING_INTERVAL_S`].
    pub interval_s: u64,
    /// Per-device SNMP timeout in milliseconds.
    pub snmp_timeout_ms: u64,
    /// Consecutive failures before an alert is raised.
    pub failures_before_alert: u32,
    /// Default per-model trust factor applied to the winning capacity
    /// source (PHY or per-station rate). `1.0` uses the reported value
    /// as-is; lower values derate a family. Must satisfy `0 < value <= 1`.
    pub capacity_factor_default: f64,
    /// SNMP community. Empty means fall back to the existing global
    /// `[sonar_integration].snmp_community`.
    pub community: String,
    /// SNMP version: `2` = v2c, `3` = v3.
    pub snmp_version: u8,
    /// Optional CSV/YAML device list for CRMs with no infrastructure
    /// inventory. Empty means no explicit list.
    pub explicit_devices_file: String,
    /// Per-model capacity trust factors, keyed by the model string the
    /// device reports. A missing key falls back to
    /// `capacity_factor_default` (1.0, used as-is). Each value must satisfy
    /// `0 < value <= 1`.
    pub capacity_factor: BTreeMap<String, f64>,
    /// Pins a device to a shipped adapter when detection cannot identify
    /// it (typically a generic `sysObjectID`). Keys are the integration
    /// node id or the management IP; values are adapter document ids.
    pub device_adapter: BTreeMap<String, String>,
    /// SNMPv3 username (used only when `snmp_version = 3`).
    pub v3_user: String,
    /// SNMPv3 authentication protocol (used only when `snmp_version = 3`).
    pub v3_auth: String,
    /// SNMPv3 authentication passphrase (used only when `snmp_version = 3`).
    pub v3_auth_pass: String,
    /// SNMPv3 privacy protocol (used only when `snmp_version = 3`).
    pub v3_priv: String,
    /// SNMPv3 privacy passphrase (used only when `snmp_version = 3`).
    pub v3_priv_pass: String,
}

impl Default for ApPollingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            apply_placement: false,
            interval_s: DEFAULT_AP_POLLING_INTERVAL_S,
            snmp_timeout_ms: DEFAULT_AP_POLLING_SNMP_TIMEOUT_MS,
            failures_before_alert: DEFAULT_AP_POLLING_FAILURES_BEFORE_ALERT,
            capacity_factor_default: DEFAULT_AP_POLLING_CAPACITY_FACTOR,
            community: String::new(),
            snmp_version: 2,
            explicit_devices_file: String::new(),
            capacity_factor: BTreeMap::new(),
            device_adapter: BTreeMap::new(),
            v3_user: String::new(),
            v3_auth: default_v3_auth(),
            v3_auth_pass: String::new(),
            v3_priv: default_v3_priv(),
            v3_priv_pass: String::new(),
        }
    }
}

fn validate_factor(field: &str, factor: f64) -> Result<(), String> {
    if !factor.is_finite() || factor <= 0.0 || factor > 1.0 {
        return Err(format!("{field} must satisfy 0 < value <= 1"));
    }
    Ok(())
}

impl ApPollingConfig {
    /// Returns true when every field equals the section default.
    pub(crate) fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// Validates the AP polling section.
    ///
    /// Rejects an interval below [`MIN_AP_POLLING_INTERVAL_S`], a capacity
    /// factor outside `0 < value <= 1`, and an SNMP version other than
    /// v2c (`2`) or v3 (`3`).
    pub fn validate(&self) -> Result<(), String> {
        if self.interval_s < MIN_AP_POLLING_INTERVAL_S {
            return Err(format!(
                "ap_polling.interval_s must be >= {MIN_AP_POLLING_INTERVAL_S}"
            ));
        }
        validate_factor(
            "ap_polling.capacity_factor_default",
            self.capacity_factor_default,
        )?;
        for (model, factor) in &self.capacity_factor {
            validate_factor(&format!("ap_polling.capacity_factor[{model:?}]"), *factor)?;
        }
        for (device, adapter) in &self.device_adapter {
            if device.trim().is_empty() {
                return Err(
                    "ap_polling.device_adapter keys must be a node id or management IP".to_string(),
                );
            }
            if adapter.trim().is_empty() {
                return Err(format!(
                    "ap_polling.device_adapter[{device:?}] must name an adapter id"
                ));
            }
        }
        if !matches!(self.snmp_version, 2 | 3) {
            return Err("ap_polling.snmp_version must be 2 (v2c) or 3 (v3)".to_string());
        }
        Ok(())
    }

    /// Resolves the effective SNMP community.
    ///
    /// A blank/whitespace-only `community` falls back to `global`, which is
    /// the existing `[sonar_integration].snmp_community`.
    pub fn effective_community<'a>(&'a self, global: &'a str) -> &'a str {
        let configured = self.community.trim();
        if configured.is_empty() {
            global
        } else {
            configured
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn parse_section(raw: &str) -> ApPollingConfig {
        toml::from_str(raw).expect("ap_polling section parses")
    }

    #[test]
    fn defaults_parse() {
        let cfg = parse_section("");
        assert!(!cfg.enabled);
        assert!(!cfg.apply_placement);
        assert_eq!(cfg.interval_s, DEFAULT_AP_POLLING_INTERVAL_S);
        assert_eq!(cfg.snmp_timeout_ms, DEFAULT_AP_POLLING_SNMP_TIMEOUT_MS);
        assert_eq!(
            cfg.failures_before_alert,
            DEFAULT_AP_POLLING_FAILURES_BEFORE_ALERT
        );
        assert_eq!(
            cfg.capacity_factor_default,
            DEFAULT_AP_POLLING_CAPACITY_FACTOR
        );
        assert_eq!(cfg.community, "");
        assert_eq!(cfg.snmp_version, 2);
        assert_eq!(cfg.explicit_devices_file, "");
        assert!(cfg.capacity_factor.is_empty());
        assert!(cfg.device_adapter.is_empty());
        assert_eq!(cfg.v3_user, "");
        assert_eq!(cfg.v3_auth, "sha256");
        assert_eq!(cfg.v3_auth_pass, "");
        assert_eq!(cfg.v3_priv, "aes");
        assert_eq!(cfg.v3_priv_pass, "");
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn full_section_parses() {
        let mut raw = include_str!("example.toml").to_string();
        raw.push_str(
            r#"
[ap_polling]
enabled = true
apply_placement = true
interval_s = 60
snmp_timeout_ms = 1200
failures_before_alert = 5
capacity_factor_default = 0.80
community = "s3cret"
snmp_version = 3
explicit_devices_file = "/opt/libreqos/ap-devices.csv"
unknown_future_key = "tolerated"
v3_user = "poll"
v3_auth = "sha256"
v3_auth_pass = "auth-pass"
v3_priv = "aes"
v3_priv_pass = "priv-pass"

[ap_polling.capacity_factor]
"PowerBeam 5AC Gen2" = 0.65

[ap_polling.device_adapter]
"100.124.2.246" = "ltu_rocket"
"node-42" = "wave_ap"
"#,
        );
        let config = Config::load_from_string(&raw).expect("full config with ap_polling parses");
        let ap = &config.ap_polling;
        assert!(ap.enabled);
        assert!(ap.apply_placement);
        assert_eq!(ap.interval_s, 60);
        assert_eq!(ap.snmp_timeout_ms, 1200);
        assert_eq!(ap.failures_before_alert, 5);
        assert_eq!(ap.capacity_factor_default, 0.80);
        assert_eq!(ap.community, "s3cret");
        assert_eq!(ap.snmp_version, 3);
        assert_eq!(ap.explicit_devices_file, "/opt/libreqos/ap-devices.csv");
        assert_eq!(ap.capacity_factor.get("PowerBeam 5AC Gen2"), Some(&0.65));
        assert_eq!(
            ap.device_adapter.get("100.124.2.246"),
            Some(&"ltu_rocket".to_string())
        );
        assert_eq!(
            ap.device_adapter.get("node-42"),
            Some(&"wave_ap".to_string())
        );
        assert_eq!(ap.v3_user, "poll");
        assert_eq!(ap.v3_auth_pass, "auth-pass");
        assert_eq!(ap.v3_priv_pass, "priv-pass");
    }

    #[test]
    fn invalid_factor_is_rejected() {
        let cfg = ApPollingConfig {
            capacity_factor_default: 0.0,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());

        let cfg = ApPollingConfig {
            capacity_factor: BTreeMap::from([("BadModel".to_string(), 1.01)]),
            ..Default::default()
        };
        assert!(cfg.validate().is_err());

        let cfg = ApPollingConfig {
            capacity_factor: BTreeMap::from([("BadModel".to_string(), f64::NAN)]),
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn invalid_interval_and_snmp_version_are_rejected() {
        let cfg = ApPollingConfig {
            interval_s: MIN_AP_POLLING_INTERVAL_S - 1,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());

        let cfg = ApPollingConfig {
            interval_s: MIN_AP_POLLING_INTERVAL_S,
            snmp_version: 1,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());

        let cfg = ApPollingConfig {
            interval_s: MIN_AP_POLLING_INTERVAL_S,
            snmp_version: 4,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn blank_device_adapter_pins_are_rejected() {
        let cfg = ApPollingConfig {
            device_adapter: BTreeMap::from([(String::new(), "ltu_rocket".to_string())]),
            ..Default::default()
        };
        assert!(cfg.validate().is_err());

        let cfg = ApPollingConfig {
            device_adapter: BTreeMap::from([("100.124.2.246".to_string(), "  ".to_string())]),
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn empty_community_falls_back_to_global() {
        let mut config =
            Config::load_from_string(include_str!("example.toml")).expect("example config parses");
        assert_eq!(config.ap_polling.community, "");
        assert_eq!(
            config.ap_polling_community(),
            config.sonar_integration.snmp_community.as_str()
        );

        config.ap_polling.community = "custom-community".to_string();
        assert_eq!(config.ap_polling_community(), "custom-community");

        config.ap_polling.community = "   ".to_string();
        assert_eq!(
            config.ap_polling_community(),
            config.sonar_integration.snmp_community.as_str()
        );
    }

    #[test]
    fn missing_section_is_disabled() {
        let config =
            Config::load_from_string(include_str!("example.toml")).expect("example config parses");
        assert!(!config.ap_polling.enabled);
        assert_eq!(config.ap_polling, ApPollingConfig::default());
    }
}
