//! Typed `device_ref` metadata emitted by integrations on infrastructure nodes.
//!
//! The Python graph payload carries the reference in the legacy `network.json`
//! node object. This module gives the runtime a typed view without changing the
//! canonical topology schema: the compatibility tree stays the source, and the
//! accessor is read-only.

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::IpAddr;

/// A per-node poller reference. All optional fields are absent when the
/// integration could not supply them; a malformed IP is absent, never an
/// error and never a fabricated address.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeviceRef {
    /// Stable integration-minted node id.
    #[serde(default, deserialize_with = "deserialize_string_or_empty")]
    pub node_id: String,
    /// Management address, when it parsed as an IP.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_ip"
    )]
    pub mgmt_ip: Option<IpAddr>,
    /// Vendor hint from the integration, when known.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_text"
    )]
    pub vendor_hint: Option<String>,
    /// Device model from the integration, when known.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_text"
    )]
    pub model: Option<String>,
    /// Expected SNMP family hint, when known.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_text"
    )]
    pub family_hint: Option<String>,
}

fn deserialize_string_or_empty<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<Value>::deserialize(deserializer)?
        .and_then(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default())
}

fn deserialize_optional_text<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(
        Option::<Value>::deserialize(deserializer)?.and_then(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ToOwned::to_owned)
        }),
    )
}

fn deserialize_optional_ip<'de, D>(deserializer: D) -> Result<Option<IpAddr>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<Value>::deserialize(deserializer)?
        .and_then(|value| value.as_str().and_then(|text| text.trim().parse().ok())))
}
