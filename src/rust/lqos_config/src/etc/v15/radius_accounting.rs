//! RADIUS accounting configuration.
//!
//! This module only defines and validates operator configuration. Runtime
//! listener startup, packet authentication, and session handling live outside
//! this config schema.

use allocative::Allocative;
use ip_network::IpNetwork;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use thiserror::Error;

fn default_ttl_seconds() -> u64 {
    900
}

fn default_stale_grace_seconds() -> u64 {
    120
}

/// Configured `secret_file` value for a RADIUS shared secret.
///
/// This stores the operator-configured `secret_file` string, normally a file
/// path. Serde preserves the value for `/etc/lqos.conf` round trips, while
/// `Debug` redacts this wrapper.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq, Allocative)]
#[serde(transparent)]
pub struct RadiusSharedSecretSource(String);

impl RadiusSharedSecretSource {
    /// Creates a wrapper for an operator-configured `secret_file` value.
    pub fn new(source: impl Into<String>) -> Self {
        Self(source.into())
    }

    /// Returns the configured `secret_file` value.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns true when the configured `secret_file` value is blank.
    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Debug for RadiusSharedSecretSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RadiusSharedSecretSource(REDACTED)")
    }
}

impl From<String> for RadiusSharedSecretSource {
    fn from(source: String) -> Self {
        Self::new(source)
    }
}

impl From<&str> for RadiusSharedSecretSource {
    fn from(source: &str) -> Self {
        Self::new(source)
    }
}

/// One trusted RADIUS client source address or network.
#[derive(Clone, Debug, PartialEq, Allocative)]
pub struct RadiusClientSource {
    #[allocative(skip)]
    network: IpNetwork,
}

impl RadiusClientSource {
    /// Creates a RADIUS client source from a parsed IP network.
    pub fn new(network: IpNetwork) -> Self {
        Self { network }
    }

    /// Returns the parsed source network.
    pub fn network(&self) -> &IpNetwork {
        &self.network
    }
}

impl Serialize for RadiusClientSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.network.to_string())
    }
}

impl<'de> Deserialize<'de> for RadiusClientSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        parse_client_source(&raw)
            .map(Self::new)
            .map_err(serde::de::Error::custom)
    }
}

/// One trusted RADIUS NAS client entry.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct RadiusAccountingClient {
    /// Optional operator label used for diagnostics.
    #[serde(default)]
    pub name: String,
    /// UDP source allow-list for this NAS client.
    ///
    /// TOML may provide either one string or a list of strings. Values must be
    /// IP addresses or CIDR networks.
    #[serde(default, deserialize_with = "deserialize_source_allow_list")]
    pub source: Vec<RadiusClientSource>,
    /// Configured `secret_file` value for this client's shared secret.
    #[serde(default)]
    pub secret_file: RadiusSharedSecretSource,
}

impl RadiusAccountingClient {
    /// Validates one trusted RADIUS client definition.
    pub fn validate(&self, index: usize) -> Result<(), String> {
        let label = client_label(index, &self.name);

        if self.source.is_empty() {
            return Err(format!(
                "{label}.source must include at least one IP address or CIDR"
            ));
        }

        if self.secret_file.is_empty() {
            return Err(format!("{label}.secret_file must not be empty"));
        }

        Ok(())
    }
}

/// Optional fallback speed profile for sessions without decoded packet rates or matched ShapedDevices rates.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct RadiusFallbackSpeedProfile {
    /// Default minimum download bandwidth in Mbps.
    pub download_min_mbps: f32,
    /// Default minimum upload bandwidth in Mbps.
    pub upload_min_mbps: f32,
    /// Default maximum download bandwidth in Mbps.
    pub download_max_mbps: f32,
    /// Default maximum upload bandwidth in Mbps.
    pub upload_max_mbps: f32,
}

impl RadiusFallbackSpeedProfile {
    /// Validates all fallback speed values.
    pub fn validate(&self) -> Result<(), String> {
        validate_rate_profile_mbps(
            self.download_min_mbps,
            self.upload_min_mbps,
            self.download_max_mbps,
            self.upload_max_mbps,
        )
        .map_err(|error| error.with_config_prefix("radius_accounting.fallback_speed_profile"))
    }
}

/// Validation error for a complete Mbps rate profile.
#[derive(Clone, Copy, Debug, Error, PartialEq)]
pub enum RateProfileValidationError {
    /// A rate value was NaN or infinite.
    #[error("{field} must be finite, got {rate_mbps} Mbps")]
    NonFinite {
        /// Field that failed validation.
        field: &'static str,
        /// Invalid rate value in Mbps.
        rate_mbps: f32,
    },
    /// A rate value was zero or negative.
    #[error("{field} must be > 0, got {rate_mbps} Mbps")]
    NonPositive {
        /// Field that failed validation.
        field: &'static str,
        /// Invalid rate value in Mbps.
        rate_mbps: f32,
    },
    /// A minimum rate exceeded its matching maximum rate.
    #[error("{min_field} ({min_mbps} Mbps) must be <= {max_field} ({max_mbps} Mbps)")]
    MinimumExceedsMaximum {
        /// Minimum-rate field that failed validation.
        min_field: &'static str,
        /// Minimum-rate value in Mbps.
        min_mbps: f32,
        /// Maximum-rate field that failed validation.
        max_field: &'static str,
        /// Maximum-rate value in Mbps.
        max_mbps: f32,
    },
}

impl RateProfileValidationError {
    /// Formats this validation error with a config-path prefix.
    #[must_use]
    pub fn with_config_prefix(&self, prefix: &str) -> String {
        match self {
            Self::NonFinite { field, .. } => format!("{prefix}.{field} must be finite"),
            Self::NonPositive { field, .. } => format!("{prefix}.{field} must be > 0"),
            Self::MinimumExceedsMaximum {
                min_field,
                max_field,
                ..
            } => format!("{prefix}.{min_field} must be <= {prefix}.{max_field}"),
        }
    }
}

/// Validates a complete Mbps rate profile.
pub fn validate_rate_profile_mbps(
    download_min_mbps: f32,
    upload_min_mbps: f32,
    download_max_mbps: f32,
    upload_max_mbps: f32,
) -> Result<(), RateProfileValidationError> {
    validate_rate_profile_field("download_min_mbps", download_min_mbps)?;
    validate_rate_profile_field("upload_min_mbps", upload_min_mbps)?;
    validate_rate_profile_field("download_max_mbps", download_max_mbps)?;
    validate_rate_profile_field("upload_max_mbps", upload_max_mbps)?;
    validate_rate_profile_order(
        "download_min_mbps",
        download_min_mbps,
        "download_max_mbps",
        download_max_mbps,
    )?;
    validate_rate_profile_order(
        "upload_min_mbps",
        upload_min_mbps,
        "upload_max_mbps",
        upload_max_mbps,
    )
}

/// RADIUS accounting listener configuration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct RadiusAccountingConfig {
    /// Persisted operator flag for RADIUS accounting listener support.
    ///
    /// When true, `lqosd` starts the listener after configuration validation.
    #[serde(default)]
    pub enabled: bool,
    /// UDP socket address for the listener, for example `0.0.0.0:1813`.
    #[allocative(skip)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen: Option<SocketAddr>,
    /// Time-to-live in seconds for accounting-derived session state.
    #[serde(default = "default_ttl_seconds")]
    pub default_ttl_seconds: u64,
    /// Grace period in seconds before stale accounting state is considered expired.
    #[serde(default = "default_stale_grace_seconds")]
    pub stale_grace_seconds: u64,
    /// Safety gate for RADIUS dynamic-circuit application.
    #[serde(default)]
    pub dynamic_circuit_application: RadiusDynamicCircuitApplicationConfig,
    /// Optional fallback speed profile for sessions without decoded packet rates or matched ShapedDevices rates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_speed_profile: Option<RadiusFallbackSpeedProfile>,
    /// Trusted RADIUS NAS clients.
    #[serde(default)]
    pub clients: Vec<RadiusAccountingClient>,
}

/// RADIUS dynamic-circuit application settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Allocative)]
pub struct RadiusDynamicCircuitApplicationConfig {
    /// Enable the RADIUS dynamic-circuit application gate.
    #[serde(default)]
    pub enabled: bool,
    /// Match `Calling-Station-Id` values to `ShapedDevices.csv` MAC fields.
    #[serde(default)]
    pub match_shaped_devices_by_mac: bool,
    /// Match RADIUS `User-Name` values to `ShapedDevices.csv` MAC fields verbatim.
    #[serde(default)]
    pub match_shaped_devices_by_username: bool,
    /// Parent node used for default RADIUS identities when MAC matching does not supply metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_parent_node: Option<String>,
    /// Stable parent node ID used with `fallback_parent_node`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_parent_node_id: Option<String>,
    /// Stable anchor node ID used with `fallback_parent_node`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_anchor_node_id: Option<String>,
}

impl RadiusDynamicCircuitApplicationConfig {
    /// Validates RADIUS dynamic-circuit application settings.
    pub fn validate(&self) -> Result<(), String> {
        validate_optional_text(
            "radius_accounting.dynamic_circuit_application.fallback_parent_node",
            self.fallback_parent_node.as_deref(),
        )?;
        validate_optional_text(
            "radius_accounting.dynamic_circuit_application.fallback_parent_node_id",
            self.fallback_parent_node_id.as_deref(),
        )?;
        validate_optional_text(
            "radius_accounting.dynamic_circuit_application.fallback_anchor_node_id",
            self.fallback_anchor_node_id.as_deref(),
        )?;

        if self.fallback_parent_node.is_none()
            && (self.fallback_parent_node_id.is_some() || self.fallback_anchor_node_id.is_some())
        {
            return Err(
                "radius_accounting.dynamic_circuit_application.fallback_parent_node must be set when fallback parent IDs are configured"
                    .to_string(),
            );
        }

        Ok(())
    }
}

impl Default for RadiusAccountingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: None,
            default_ttl_seconds: default_ttl_seconds(),
            stale_grace_seconds: default_stale_grace_seconds(),
            dynamic_circuit_application: RadiusDynamicCircuitApplicationConfig::default(),
            fallback_speed_profile: None,
            clients: Vec::new(),
        }
    }
}

impl RadiusAccountingConfig {
    /// Validates RADIUS accounting configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.default_ttl_seconds == 0 {
            return Err("radius_accounting.default_ttl_seconds must be > 0".to_string());
        }
        if self.stale_grace_seconds == 0 {
            return Err("radius_accounting.stale_grace_seconds must be > 0".to_string());
        }

        if self.listen.is_some_and(|listen| listen.port() == 0) {
            return Err("radius_accounting.listen port must be > 0".to_string());
        }

        if self.dynamic_circuit_application.enabled {
            self.dynamic_circuit_application.validate()?;
            if let Some(fallback_speed_profile) = &self.fallback_speed_profile {
                fallback_speed_profile.validate()?;
            }
        }

        for (index, client) in self.clients.iter().enumerate() {
            client.validate(index)?;
        }

        if self.enabled {
            if self.listen.is_none() {
                return Err("radius_accounting.listen must be configured when enabled".to_string());
            }
            if self.clients.is_empty() {
                return Err(
                    "radius_accounting.clients must include at least one client when enabled"
                        .to_string(),
                );
            }
        }

        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SourceAllowList {
    One(String),
    Many(Vec<String>),
}

fn deserialize_source_allow_list<'de, D>(
    deserializer: D,
) -> Result<Vec<RadiusClientSource>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match SourceAllowList::deserialize(deserializer)? {
        SourceAllowList::One(source) => parse_source_allow_list(vec![source]),
        SourceAllowList::Many(sources) => parse_source_allow_list(sources),
    }
}

fn parse_source_allow_list<E>(sources: Vec<String>) -> Result<Vec<RadiusClientSource>, E>
where
    E: serde::de::Error,
{
    sources
        .into_iter()
        .map(|source| {
            parse_client_source(&source)
                .map(RadiusClientSource::new)
                .map_err(serde::de::Error::custom)
        })
        .collect()
}

fn client_label(index: usize, name: &str) -> String {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        format!("radius_accounting.clients[{index}]")
    } else {
        format!("radius_accounting.clients[{index}] ('{trimmed_name}')")
    }
}

fn validate_optional_text(config_path: &str, value: Option<&str>) -> Result<(), String> {
    if value.is_some_and(|value| value.trim().is_empty()) {
        return Err(format!("{config_path} must not be empty when configured"));
    }
    Ok(())
}

fn validate_rate_profile_field(
    field: &'static str,
    rate_mbps: f32,
) -> Result<(), RateProfileValidationError> {
    if !rate_mbps.is_finite() {
        return Err(RateProfileValidationError::NonFinite { field, rate_mbps });
    }
    if rate_mbps <= 0.0 {
        return Err(RateProfileValidationError::NonPositive { field, rate_mbps });
    }
    Ok(())
}

fn validate_rate_profile_order(
    min_field: &'static str,
    min_mbps: f32,
    max_field: &'static str,
    max_mbps: f32,
) -> Result<(), RateProfileValidationError> {
    if min_mbps > max_mbps {
        return Err(RateProfileValidationError::MinimumExceedsMaximum {
            min_field,
            min_mbps,
            max_field,
            max_mbps,
        });
    }
    Ok(())
}

fn parse_client_source(source: &str) -> Result<IpNetwork, String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("source is empty".to_string());
    }

    if let Ok(network) = trimmed.parse::<IpNetwork>() {
        return Ok(network);
    }

    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Ok(IpNetwork::from(ip));
    }

    Err(format!("'{trimmed}' is not a valid IP address or CIDR"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::etc::v15::Config;

    const TEST_CLIENT_NAME: &str = "pppoe-core-1";
    const TEST_CLIENT_NAME_LINE: &str = "name = \"pppoe-core-1\"";
    const TEST_LISTEN: &str = "127.0.0.1:18130";
    const TEST_LISTEN_LINE: &str = "listen = \"127.0.0.1:18130\"";
    const TEST_SOURCE: &str = "192.0.2.10/32";
    const TEST_SOURCE_LINE: &str = "source = \"192.0.2.10/32\"";
    const TEST_SECRET_FILE: &str = "/etc/lqos/radius-secrets/pppoe-core-1";
    const TEST_SECRET_FILE_LINE: &str = "secret_file = \"/etc/lqos/radius-secrets/pppoe-core-1\"";
    const TEST_TTL_SECONDS_LINE: &str = "default_ttl_seconds = 900";
    const TEST_STALE_GRACE_SECONDS_LINE: &str = "stale_grace_seconds = 120";
    const TEST_FALLBACK_SECTION_LINES: &[&str] = &[
        "[radius_accounting.fallback_speed_profile]",
        "download_min_mbps = 5.0",
        "upload_min_mbps = 3.0",
        "download_max_mbps = 25.0",
        "upload_max_mbps = 10.0",
    ];
    const TEST_CLIENT_LINES: &[&str] = &[TEST_SOURCE_LINE, TEST_SECRET_FILE_LINE];
    const TEST_NAMED_CLIENT_LINES: &[&str] = &[
        TEST_CLIENT_NAME_LINE,
        TEST_SOURCE_LINE,
        TEST_SECRET_FILE_LINE,
    ];

    fn source(raw: &str) -> RadiusClientSource {
        RadiusClientSource::new(parse_client_source(raw).expect("test source should parse"))
    }

    fn radius_section(
        enabled: bool,
        section_lines: &[&str],
        client_lines: Option<&[&str]>,
    ) -> String {
        let mut radius = format!("\n\n[radius_accounting]\nenabled = {enabled}\n");
        for line in section_lines {
            radius.push_str(line);
            radius.push('\n');
        }
        if let Some(client_lines) = client_lines {
            radius.push_str("\n[[radius_accounting.clients]]\n");
            for line in client_lines {
                radius.push_str(line);
                radius.push('\n');
            }
        }
        radius
    }

    fn enabled_radius_section(section_lines: &[&str], client_lines: Option<&[&str]>) -> String {
        let mut lines = Vec::with_capacity(section_lines.len() + 1);
        lines.push(TEST_LISTEN_LINE);
        lines.extend_from_slice(section_lines);
        radius_section(true, &lines, client_lines)
    }

    fn enabled_radius_section_with_clients(client_sections: &[&[&str]]) -> String {
        let mut radius = format!("\n\n[radius_accounting]\nenabled = true\n{TEST_LISTEN_LINE}\n");
        for client_lines in client_sections {
            radius.push_str("\n[[radius_accounting.clients]]\n");
            for line in *client_lines {
                radius.push_str(line);
                radius.push('\n');
            }
        }
        radius
    }

    fn example_with_radius(radius: &str) -> String {
        let mut raw = include_str!("example.toml").to_string();
        raw.push_str(radius);
        raw
    }

    fn load_error_contains(radius: &str, expected: &str) {
        let raw = example_with_radius(radius);
        let error = Config::load_from_string(&raw).expect_err("config should fail validation");
        assert!(
            error.contains(expected),
            "expected error to contain '{expected}', got '{error}'"
        );
    }

    fn first_radius_client(radius: &str) -> RadiusAccountingClient {
        Config::load_from_string(&example_with_radius(radius))
            .expect("radius accounting should deserialize")
            .radius_accounting
            .expect("radius accounting should be present")
            .clients
            .into_iter()
            .next()
            .expect("radius accounting should include a client")
    }

    fn load_error_cases(cases: impl IntoIterator<Item = (String, &'static str)>) {
        for (radius, expected) in cases {
            load_error_contains(&radius, expected);
        }
    }

    fn assert_disabled_defaults(radius: &RadiusAccountingConfig) {
        assert!(!radius.enabled);
        assert!(radius.listen.is_none());
        assert!(radius.clients.is_empty());
        assert_eq!(radius.default_ttl_seconds, default_ttl_seconds());
        assert_eq!(radius.stale_grace_seconds, default_stale_grace_seconds());
        assert!(!radius.dynamic_circuit_application.enabled);
        assert!(
            !radius
                .dynamic_circuit_application
                .match_shaped_devices_by_mac
        );
        assert!(
            radius
                .dynamic_circuit_application
                .fallback_parent_node
                .is_none()
        );
        assert!(
            radius
                .dynamic_circuit_application
                .fallback_parent_node_id
                .is_none()
        );
        assert!(
            radius
                .dynamic_circuit_application
                .fallback_anchor_node_id
                .is_none()
        );
        assert!(radius.fallback_speed_profile.is_none());
    }

    fn valid_client() -> RadiusAccountingClient {
        RadiusAccountingClient {
            name: TEST_CLIENT_NAME.to_string(),
            source: vec![source(TEST_SOURCE), source("2001:db8::/48")],
            secret_file: RadiusSharedSecretSource::from(TEST_SECRET_FILE),
        }
    }

    fn valid_fallback_speed_profile() -> RadiusFallbackSpeedProfile {
        RadiusFallbackSpeedProfile {
            download_min_mbps: 5.0,
            upload_min_mbps: 3.0,
            download_max_mbps: 25.0,
            upload_max_mbps: 10.0,
        }
    }

    fn valid_enabled_config() -> RadiusAccountingConfig {
        RadiusAccountingConfig {
            enabled: true,
            listen: Some(
                TEST_LISTEN
                    .parse()
                    .expect("test listen address should parse"),
            ),
            default_ttl_seconds: default_ttl_seconds(),
            stale_grace_seconds: default_stale_grace_seconds(),
            dynamic_circuit_application: RadiusDynamicCircuitApplicationConfig::default(),
            fallback_speed_profile: None,
            clients: vec![valid_client()],
        }
    }

    #[test]
    fn disabled_radius_accounting_allows_no_clients() {
        let config = RadiusAccountingConfig::default();

        config
            .validate()
            .expect("disabled radius accounting should not require clients");
    }

    #[test]
    fn disabled_radius_accounting_round_trips() {
        let config = Config {
            radius_accounting: Some(RadiusAccountingConfig::default()),
            ..Config::default()
        };

        let raw = toml::to_string_pretty(&config).expect("config should serialize");
        let parsed = Config::load_from_string(&raw).expect("config should deserialize");
        let radius = parsed
            .radius_accounting
            .expect("radius accounting section should round trip");

        assert_disabled_defaults(&radius);
    }

    #[test]
    fn empty_radius_accounting_toml_section_uses_disabled_defaults() {
        let config = Config::load_from_string(&example_with_radius(
            r#"

[radius_accounting]
"#,
        ))
        .expect("empty radius accounting section should deserialize");
        let radius = config
            .radius_accounting
            .expect("radius accounting should be present");

        assert_disabled_defaults(&radius);
    }

    #[test]
    fn enabled_radius_accounting_round_trips_and_redacts_debug_secret_source() {
        let config = Config {
            radius_accounting: Some(valid_enabled_config()),
            ..Config::default()
        };

        config
            .validate()
            .expect("radius accounting should validate");
        let debug = format!("{config:?}");
        assert!(!debug.contains(TEST_SECRET_FILE));
        assert!(debug.contains("RadiusSharedSecretSource(REDACTED)"));

        let raw = toml::to_string_pretty(&config).expect("config should serialize");
        assert!(raw.contains("[radius_accounting]"));
        assert!(raw.contains(TEST_SECRET_FILE_LINE));

        let parsed = Config::load_from_string(&raw).expect("config should deserialize");
        assert_eq!(parsed.radius_accounting, config.radius_accounting);

        let toml_config = Config::load_from_string(&example_with_radius(&enabled_radius_section(
            TEST_FALLBACK_SECTION_LINES,
            Some(TEST_CLIENT_LINES),
        )))
        .expect("fallback speed profile TOML should deserialize");
        assert_eq!(
            toml_config
                .radius_accounting
                .expect("radius accounting should be present")
                .fallback_speed_profile,
            Some(valid_fallback_speed_profile())
        );
    }

    #[test]
    fn disabled_dynamic_circuit_application_round_trips() {
        let mut radius = valid_enabled_config();
        radius.dynamic_circuit_application.enabled = false;
        let config = Config {
            radius_accounting: Some(radius),
            ..Config::default()
        };

        let raw = toml::to_string_pretty(&config).expect("config should serialize");
        let parsed = Config::load_from_string(&raw).expect("config should deserialize");
        let radius = parsed
            .radius_accounting
            .expect("radius accounting section should round trip");

        assert!(!radius.dynamic_circuit_application.enabled);
    }

    #[test]
    fn enabled_dynamic_circuit_application_round_trips_without_top_level_dynamic_circuits() {
        let mut radius = valid_enabled_config();
        radius.dynamic_circuit_application.enabled = true;
        let config = Config {
            radius_accounting: Some(radius),
            ..Config::default()
        };

        config
            .validate()
            .expect("valid RADIUS dynamic circuit activation should pass");
        let raw = toml::to_string_pretty(&config).expect("config should serialize");
        assert!(raw.contains("[radius_accounting.dynamic_circuit_application]"));

        let parsed = Config::load_from_string(&raw).expect("config should deserialize");
        let radius = parsed
            .radius_accounting
            .expect("radius accounting section should round trip");
        assert!(radius.dynamic_circuit_application.enabled);
        assert!(
            !radius
                .dynamic_circuit_application
                .match_shaped_devices_by_mac
        );
        assert!(parsed.dynamic_circuits.is_none());
    }

    #[test]
    fn fallback_speed_profile_round_trips() {
        let mut radius = valid_enabled_config();
        radius.fallback_speed_profile = Some(valid_fallback_speed_profile());
        let config = Config {
            radius_accounting: Some(radius),
            ..Config::default()
        };

        config
            .validate()
            .expect("valid fallback speed profile should pass");
        let raw = toml::to_string_pretty(&config).expect("config should serialize");
        assert!(raw.contains("[radius_accounting.fallback_speed_profile]"));
        assert!(raw.contains("download_min_mbps = 5.0"));

        let parsed = Config::load_from_string(&raw).expect("config should deserialize");
        assert_eq!(parsed.radius_accounting, config.radius_accounting);
    }

    #[test]
    fn mac_match_dynamic_application_setting_round_trips() {
        let mut radius = valid_enabled_config();
        radius.dynamic_circuit_application.enabled = true;
        radius
            .dynamic_circuit_application
            .match_shaped_devices_by_mac = true;
        let config = Config {
            radius_accounting: Some(radius),
            ..Config::default()
        };

        config
            .validate()
            .expect("valid RADIUS MAC matching settings should pass");
        let raw = toml::to_string_pretty(&config).expect("config should serialize");
        assert!(raw.contains("match_shaped_devices_by_mac = true"));

        let parsed = Config::load_from_string(&raw).expect("config should deserialize");
        let radius = parsed
            .radius_accounting
            .expect("radius accounting section should round trip");
        assert!(
            radius
                .dynamic_circuit_application
                .match_shaped_devices_by_mac
        );
    }

    #[test]
    fn username_match_dynamic_application_setting_round_trips() {
        let mut radius = valid_enabled_config();
        radius.dynamic_circuit_application.enabled = true;
        radius
            .dynamic_circuit_application
            .match_shaped_devices_by_username = true;
        let config = Config {
            radius_accounting: Some(radius),
            ..Config::default()
        };

        config
            .validate()
            .expect("valid RADIUS username matching settings should pass");
        let raw = toml::to_string_pretty(&config).expect("config should serialize");
        assert!(raw.contains("match_shaped_devices_by_username = true"));

        let parsed = Config::load_from_string(&raw).expect("config should deserialize");
        let radius = parsed
            .radius_accounting
            .expect("radius accounting section should round trip");
        assert!(
            radius
                .dynamic_circuit_application
                .match_shaped_devices_by_username
        );
    }

    #[test]
    fn fallback_parent_settings_round_trip() {
        let mut radius = valid_enabled_config();
        radius.dynamic_circuit_application.enabled = true;
        radius.dynamic_circuit_application.fallback_parent_node = Some("Core PPPoE".to_string());
        radius.dynamic_circuit_application.fallback_parent_node_id = Some("core-pppoe".to_string());
        radius.dynamic_circuit_application.fallback_anchor_node_id =
            Some("radius-anchor".to_string());
        let config = Config {
            radius_accounting: Some(radius),
            ..Config::default()
        };

        config
            .validate()
            .expect("valid RADIUS fallback parent settings should pass");
        let raw = toml::to_string_pretty(&config).expect("config should serialize");
        assert!(raw.contains("fallback_parent_node = \"Core PPPoE\""));
        assert!(raw.contains("fallback_parent_node_id = \"core-pppoe\""));
        assert!(raw.contains("fallback_anchor_node_id = \"radius-anchor\""));

        let parsed = Config::load_from_string(&raw).expect("config should deserialize");
        assert_eq!(parsed.radius_accounting, config.radius_accounting);
    }

    #[test]
    fn validation_rejects_invalid_fallback_speed_profile() {
        for (label, profile, expected) in [
            (
                "non-finite download minimum",
                RadiusFallbackSpeedProfile {
                    download_min_mbps: f32::NAN,
                    ..valid_fallback_speed_profile()
                },
                "download_min_mbps must be finite",
            ),
            (
                "non-finite upload maximum",
                RadiusFallbackSpeedProfile {
                    upload_max_mbps: f32::INFINITY,
                    ..valid_fallback_speed_profile()
                },
                "upload_max_mbps must be finite",
            ),
            (
                "zero download maximum",
                RadiusFallbackSpeedProfile {
                    download_max_mbps: 0.0,
                    ..valid_fallback_speed_profile()
                },
                "download_max_mbps must be > 0",
            ),
            (
                "negative upload minimum",
                RadiusFallbackSpeedProfile {
                    upload_min_mbps: -1.0,
                    ..valid_fallback_speed_profile()
                },
                "upload_min_mbps must be > 0",
            ),
            (
                "download minimum greater than maximum",
                RadiusFallbackSpeedProfile {
                    download_min_mbps: 30.0,
                    ..valid_fallback_speed_profile()
                },
                "download_min_mbps must be <= radius_accounting.fallback_speed_profile.download_max_mbps",
            ),
            (
                "upload minimum greater than maximum",
                RadiusFallbackSpeedProfile {
                    upload_min_mbps: 15.0,
                    ..valid_fallback_speed_profile()
                },
                "upload_min_mbps must be <= radius_accounting.fallback_speed_profile.upload_max_mbps",
            ),
        ] {
            let mut config = valid_enabled_config();
            config.dynamic_circuit_application.enabled = true;
            config.fallback_speed_profile = Some(profile);
            let error = config.validate().expect_err(label);

            assert!(
                error.contains(expected),
                "expected error to contain '{expected}', got '{error}'"
            );
        }
    }

    #[test]
    fn dynamic_circuit_application_does_not_require_top_level_dynamic_circuits() {
        let mut radius = valid_enabled_config();
        radius.dynamic_circuit_application.enabled = true;

        let missing_top_level_dynamic_circuits = Config {
            radius_accounting: Some(radius.clone()),
            ..Config::default()
        };
        missing_top_level_dynamic_circuits
            .validate()
            .expect("RADIUS dynamic circuit activation should not require dynamic_circuits");

        let disabled_top_level_dynamic_circuits = Config {
            dynamic_circuits: Some(Default::default()),
            radius_accounting: Some(radius),
            ..Config::default()
        };
        disabled_top_level_dynamic_circuits
            .validate()
            .expect("disabled dynamic_circuits should not block RADIUS dynamic circuit activation");
    }

    #[test]
    fn disabled_radius_accounting_dynamic_application_does_not_require_dynamic_circuits() {
        let radius = RadiusAccountingConfig {
            dynamic_circuit_application: RadiusDynamicCircuitApplicationConfig {
                enabled: true,
                ..RadiusDynamicCircuitApplicationConfig::default()
            },
            ..RadiusAccountingConfig::default()
        };
        let config = Config {
            radius_accounting: Some(radius),
            ..Config::default()
        };

        config
            .validate()
            .expect("disabled RADIUS accounting should not require dynamic_circuits");
    }

    #[test]
    fn dynamic_circuit_activation_settings_deserialize_without_top_level_dynamic_circuits() {
        let radius = format!(
            r#"

[radius_accounting]
enabled = true
{TEST_LISTEN_LINE}

[radius_accounting.dynamic_circuit_application]
enabled = true

[[radius_accounting.clients]]
{TEST_SOURCE_LINE}
{TEST_SECRET_FILE_LINE}
"#
        );
        let config = Config::load_from_string(&example_with_radius(&radius))
            .expect("valid activation settings should deserialize");
        let radius = config
            .radius_accounting
            .expect("radius accounting should be present");

        assert!(radius.dynamic_circuit_application.enabled);
        assert!(config.dynamic_circuits.is_none());
    }

    #[test]
    fn source_accepts_single_string_in_toml() {
        let radius = enabled_radius_section(
            &[TEST_TTL_SECONDS_LINE, TEST_STALE_GRACE_SECONDS_LINE],
            Some(TEST_NAMED_CLIENT_LINES),
        );
        let client = first_radius_client(&radius);

        assert_eq!(client.source[0].network().to_string(), "192.0.2.10/32");
    }

    #[test]
    fn enabled_toml_without_timing_values_uses_defaults() {
        let radius = enabled_radius_section(&[], Some(TEST_CLIENT_LINES));
        let config = Config::load_from_string(&example_with_radius(&radius))
            .expect("timing defaults should deserialize");
        let radius = config
            .radius_accounting
            .expect("radius accounting should be present");

        assert_eq!(radius.default_ttl_seconds, default_ttl_seconds());
        assert_eq!(radius.stale_grace_seconds, default_stale_grace_seconds());
    }

    #[test]
    fn enabled_radius_accounting_requires_listen_and_clients() {
        let mut config = valid_enabled_config();
        config.listen = None;
        let error = config
            .validate()
            .expect_err("enabled radius accounting should require listen");
        assert!(error.contains("listen"));

        let mut config = valid_enabled_config();
        config.clients.clear();
        let error = config
            .validate()
            .expect_err("enabled radius accounting should require clients");
        assert!(error.contains("clients"));
    }

    #[test]
    fn validation_rejects_invalid_listen_address() {
        let radius = radius_section(
            true,
            &[r#"listen = "not-a-socket""#],
            Some(TEST_CLIENT_LINES),
        );
        load_error_contains(&radius, "invalid socket address");
    }

    #[test]
    fn validation_rejects_zero_listen_port() {
        let mut config = valid_enabled_config();
        config.listen = Some("0.0.0.0:0".parse().expect("test listen should parse"));

        let error = config
            .validate()
            .expect_err("zero listen port should fail validation");

        assert!(error.contains("listen port"));
    }

    #[test]
    fn validation_rejects_invalid_client_source() {
        let radius = enabled_radius_section(
            &[],
            Some(&[r#"source = "not-a-network""#, TEST_SECRET_FILE_LINE]),
        );
        load_error_contains(&radius, "not a valid IP address or CIDR");
    }

    #[test]
    fn validation_rejects_missing_secret_source() {
        let mut config = valid_enabled_config();
        config.clients[0].secret_file = RadiusSharedSecretSource::default();

        let error = config
            .validate()
            .expect_err("missing secret source should fail validation");

        assert!(error.contains("secret_file"));
    }

    #[test]
    fn validation_rejects_missing_client_source() {
        load_error_cases([
            (
                enabled_radius_section(&[], Some(&[TEST_SECRET_FILE_LINE])),
                "source",
            ),
            (
                enabled_radius_section(&[], Some(&["source = []", TEST_SECRET_FILE_LINE])),
                "source",
            ),
        ]);
    }

    #[test]
    fn validation_checks_each_configured_client() {
        let mut config = valid_enabled_config();
        let mut second_client = valid_client();
        second_client.name = "pppoe-core-2".to_string();
        second_client.source.clear();
        config.clients.push(second_client);

        let error = config
            .validate()
            .expect_err("second client missing source should fail validation");

        assert!(error.contains("clients[1]"));
        assert!(error.contains("pppoe-core-2"));
        assert!(error.contains("source"));

        let mut config = valid_enabled_config();
        let mut second_client = valid_client();
        second_client.name = "pppoe-core-2".to_string();
        second_client.secret_file = RadiusSharedSecretSource::default();
        config.clients.push(second_client);

        let error = config
            .validate()
            .expect_err("second client missing secret should fail validation");

        assert!(error.contains("clients[1]"));
        assert!(error.contains("pppoe-core-2"));
        assert!(error.contains("secret_file"));
    }

    #[test]
    fn multiple_clients_toml_deserializes_and_validates() {
        let radius = enabled_radius_section_with_clients(&[
            TEST_NAMED_CLIENT_LINES,
            &[
                r#"name = "pppoe-core-2""#,
                r#"source = ["192.0.2.11/32", "2001:db8:1::/48"]"#,
                TEST_SECRET_FILE_LINE,
            ],
        ]);
        let config = Config::load_from_string(&example_with_radius(&radius))
            .expect("multiple radius clients should deserialize");
        let clients = config
            .radius_accounting
            .expect("radius accounting should be present")
            .clients;

        assert_eq!(clients.len(), 2);
        assert_eq!(clients[1].name, "pppoe-core-2");
        assert_eq!(clients[1].source[0].network().to_string(), "192.0.2.11/32");
        assert_eq!(
            clients[1].source[1].network().to_string(),
            "2001:db8:1::/48"
        );
    }

    #[test]
    fn multiple_clients_toml_reports_second_client_validation_error() {
        let radius = enabled_radius_section_with_clients(&[
            TEST_NAMED_CLIENT_LINES,
            &[r#"name = "pppoe-core-2""#, TEST_SECRET_FILE_LINE],
        ]);
        let raw = example_with_radius(&radius);

        let error = Config::load_from_string(&raw)
            .expect_err("second TOML client missing source should fail validation");

        assert!(error.contains("clients[1]"));
        assert!(error.contains("pppoe-core-2"));
        assert!(error.contains("source"));
    }

    #[test]
    fn validation_rejects_non_positive_timing_values() {
        let mut config = valid_enabled_config();
        config.default_ttl_seconds = 0;
        let error = config
            .validate()
            .expect_err("zero ttl should fail validation");
        assert!(error.contains("default_ttl_seconds"));

        let mut config = valid_enabled_config();
        config.stale_grace_seconds = 0;
        let error = config
            .validate()
            .expect_err("zero stale grace should fail validation");
        assert!(error.contains("stale_grace_seconds"));
    }

    #[test]
    fn top_level_load_rejects_validation_failures() {
        load_error_cases([
            (
                enabled_radius_section(&[], Some(&[TEST_SOURCE_LINE])),
                "secret_file",
            ),
            (
                enabled_radius_section(&["default_ttl_seconds = 0"], Some(TEST_CLIENT_LINES)),
                "default_ttl_seconds",
            ),
            (
                enabled_radius_section(&["stale_grace_seconds = 0"], Some(TEST_CLIENT_LINES)),
                "stale_grace_seconds",
            ),
            (
                enabled_radius_section(
                    &[
                        "[radius_accounting.dynamic_circuit_application]",
                        "enabled = true",
                        "[radius_accounting.fallback_speed_profile]",
                        "download_min_mbps = 0.0",
                        "upload_min_mbps = 3.0",
                        "download_max_mbps = 25.0",
                        "upload_max_mbps = 10.0",
                    ],
                    Some(TEST_CLIENT_LINES),
                ),
                "fallback_speed_profile.download_min_mbps",
            ),
            (
                enabled_radius_section(
                    &[
                        "[radius_accounting.dynamic_circuit_application]",
                        "enabled = true",
                        "fallback_parent_node = \"   \"",
                    ],
                    Some(TEST_CLIENT_LINES),
                ),
                "fallback_parent_node",
            ),
            (
                enabled_radius_section(
                    &[
                        "[radius_accounting.dynamic_circuit_application]",
                        "enabled = true",
                        "fallback_parent_node_id = \"core-pppoe\"",
                    ],
                    Some(TEST_CLIENT_LINES),
                ),
                "fallback_parent_node must be set",
            ),
            (enabled_radius_section(&[], None), "clients"),
        ]);
    }

    #[test]
    fn disabled_radius_accounting_rejects_invalid_configured_values() {
        load_error_contains(
            r#"

[radius_accounting]
enabled = false
default_ttl_seconds = 0
"#,
            "default_ttl_seconds",
        );

        load_error_contains(
            r#"

[radius_accounting]
enabled = false
listen = "0.0.0.0:0"
"#,
            "listen port",
        );
    }

    #[test]
    fn disabled_radius_accounting_rejects_invalid_configured_clients() {
        load_error_cases([
            (
                radius_section(false, &[], Some(&["source = []", TEST_SECRET_FILE_LINE])),
                "source",
            ),
            (
                radius_section(
                    false,
                    &[],
                    Some(&[TEST_SOURCE_LINE, r#"secret_file = "   ""#]),
                ),
                "secret_file",
            ),
            (
                radius_section(
                    false,
                    &[],
                    Some(&[r#"source = "   ""#, TEST_SECRET_FILE_LINE]),
                ),
                "source is empty",
            ),
        ]);
    }

    #[test]
    fn validation_accepts_bare_ip_sources() {
        let radius = enabled_radius_section(
            &[],
            Some(&[
                r#"source = ["192.0.2.10", "2001:db8::1"]"#,
                TEST_SECRET_FILE_LINE,
            ]),
        );
        let client = first_radius_client(&radius);

        assert_eq!(client.source[0].network().to_string(), "192.0.2.10/32");
        assert_eq!(client.source[1].network().to_string(), "2001:db8::1/128");
    }
}
