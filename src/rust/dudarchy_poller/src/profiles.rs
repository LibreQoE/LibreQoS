//! Typed vault credential profiles under the optional top-level `profiles`
//! array: one name, one protocol's credentials, an optional IPv4 scope, and
//! an opt-in `discovery` flag.
//!
//! [`crate::vault::Vault`] parses the array eagerly, so a malformed entry
//! fails the load with a [`ProfileError`] naming the profile. Resolution is
//! pure and read-only: [`resolve_profiles`] returns the discovery-enabled,
//! protocol-matching, scope-matching profiles for one address, most
//! specific first (exact address, then longest prefix, then file order; an
//! absent scope matches every address at lowest specificity). No network
//! code lives here.

use std::fmt;
use std::net::Ipv4Addr;

use serde_json::{Map, Value};

use crate::snmp::SnmpVersion;

/// The wire protocol a profile authenticates: SNMP v2c or the RouterOS API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// SNMP v2c, authenticated by a community string.
    Snmp,
    /// RouterOS binary API, authenticated by username and password.
    Api,
}

/// The credentials a profile carries; the variant fixes the protocol.
///
/// Values are plain strings, but `Debug` is redacted: no variant output
/// names the community, username, or password, and no `Display`
/// implementation exists.
pub enum Credential {
    /// SNMP v2c community for [`Protocol::Snmp`].
    Snmp {
        /// Community string presented to the agent.
        community: String,
    },
    /// RouterOS API login for [`Protocol::Api`].
    Api {
        /// API user name; redacted in `Debug` like the password.
        username: String,
        /// API password.
        password: String,
    },
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snmp { .. } => f.write_str("Snmp { community: *** }"),
            Self::Api { .. } => f.write_str("Api { username: ***, password: *** }"),
        }
    }
}

/// One parsed `profiles` entry: a name, per-protocol credentials, an
/// optional IPv4 scope, and the `discovery` opt-in flag (default false).
///
/// Derived `Debug` cannot leak credentials: the only secrets live in
/// [`Credential`], whose `Debug` is redacted.
#[derive(Debug)]
pub struct Profile {
    name: String,
    credential: Credential,
    scope: Option<Vec<ScopeEntry>>,
    discovery: bool,
    snmp_version: SnmpVersion,
}

impl Profile {
    /// The profile's `name` field.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The profile's protocol, fixed by which credential fields it carries.
    pub fn protocol(&self) -> Protocol {
        match self.credential {
            Credential::Snmp { .. } => Protocol::Snmp,
            Credential::Api { .. } => Protocol::Api,
        }
    }

    /// The profile's credentials; the variant names the protocol.
    pub fn credential(&self) -> &Credential {
        &self.credential
    }

    /// Whether discovery may use this profile; the `discovery` field
    /// defaults to false.
    pub fn discovery(&self) -> bool {
        self.discovery
    }

    /// The SNMP version this profile prefers on discovery; the
    /// `snmp_version` field defaults to v2c, which discovery retries once
    /// as v1 when the v2c system read yields no usable data.
    pub fn snmp_version(&self) -> SnmpVersion {
        self.snmp_version
    }

    /// Whether this profile carries no `scope`, i.e. it matches every
    /// address: the walker's shared-default class.
    pub fn is_unscoped(&self) -> bool {
        self.scope.is_none()
    }

    /// The longest prefix length among this profile's scope entries that
    /// contain `address`: 32 for an exact address, the written prefix for a
    /// CIDR entry. An unscoped profile matches every address at specificity
    /// 0; a scoped profile without a matching entry does not match.
    fn match_specificity(&self, address: Ipv4Addr) -> Option<u8> {
        match &self.scope {
            None => Some(0),
            Some(entries) => entries
                .iter()
                .filter(|entry| entry.contains(address))
                .map(|entry| entry.prefix)
                .max(),
        }
    }
}

/// Returns the profiles that may authenticate `address` over `protocol`:
/// discovery-enabled, scope-matched profiles ordered most specific first —
/// exact address, then longest prefix, then file order. Pure and read-only.
pub fn resolve_profiles(
    profiles: &[Profile],
    address: Ipv4Addr,
    protocol: Protocol,
) -> Vec<&Profile> {
    let mut matched: Vec<(u8, usize, &Profile)> = Vec::new();
    for (index, profile) in profiles.iter().enumerate() {
        if !profile.discovery || profile.protocol() != protocol {
            continue;
        }
        let Some(specificity) = profile.match_specificity(address) else {
            continue;
        };
        matched.push((specificity, index, profile));
    }
    // Descending specificity; the file index breaks ties deterministically.
    matched.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    matched.into_iter().map(|(_, _, profile)| profile).collect()
}

/// One scope entry: an IPv4 network with a prefix length. An exact address
/// is a /32.
#[derive(Debug)]
struct ScopeEntry {
    /// The network address as written; `contains` masks both sides, so
    /// host bits in the written address do not affect matching.
    network: Ipv4Addr,
    /// Prefix length in 0..=32, validated at parse time.
    prefix: u8,
}

impl ScopeEntry {
    /// Whether `address` falls inside this entry's network.
    fn contains(&self, address: Ipv4Addr) -> bool {
        let mask = prefix_mask(self.prefix);
        u32::from(address) & mask == u32::from(self.network) & mask
    }
}

/// The netmask for a prefix length validated to 0..=32.
fn prefix_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    }
}

/// Parses raw `profiles` entries in file order; the first malformed entry
/// aborts with a [`ProfileError`] naming it.
pub(crate) fn parse_all(entries: &[Value]) -> Result<Vec<Profile>, ProfileError> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| parse_profile(entry, index))
        .collect()
}

/// Parses one raw `profiles` entry. Failures before the `name` field parses
/// report the entry's array `index`; everything after reports the name.
fn parse_profile(entry: &Value, index: usize) -> Result<Profile, ProfileError> {
    let Some(fields) = entry.as_object() else {
        return Err(ProfileError::NotAnObject { index });
    };
    let Some(name) = fields.get("name").and_then(Value::as_str) else {
        return Err(ProfileError::NoName { index });
    };
    let name = name.to_owned();
    let credential = match string_field(fields, &name, "protocol")? {
        "snmp" => Credential::Snmp {
            community: string_field(fields, &name, "community")?.to_owned(),
        },
        "api" => Credential::Api {
            username: string_field(fields, &name, "username")?.to_owned(),
            password: string_field(fields, &name, "password")?.to_owned(),
        },
        other => {
            return Err(ProfileError::UnknownProtocol {
                name,
                protocol: other.to_owned(),
            });
        }
    };
    let scope = match fields.get("scope") {
        None => None,
        Some(Value::Array(entries)) => Some(
            entries
                .iter()
                .map(|entry| parse_scope_entry(entry, &name))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err(ProfileError::ScopeNotArray { name }),
    };
    let discovery = match fields.get("discovery") {
        None => false,
        Some(Value::Bool(enabled)) => *enabled,
        Some(_) => return Err(ProfileError::DiscoveryNotBool { name }),
    };
    let snmp_version = match fields.get("snmp_version") {
        None => SnmpVersion::V2c,
        Some(Value::String(text)) => {
            SnmpVersion::parse(text).ok_or_else(|| ProfileError::BadSnmpVersion {
                name: name.clone(),
                value: text.clone(),
            })?
        }
        Some(_) => {
            return Err(ProfileError::BadSnmpVersion {
                name,
                value: fields
                    .get("snmp_version")
                    .map(Value::to_string)
                    .unwrap_or_default(),
            });
        }
    };
    Ok(Profile {
        name,
        credential,
        scope,
        discovery,
        snmp_version,
    })
}

/// Parses one scope entry: an IPv4 address (exact match, /32) or an
/// `address/prefix` CIDR range.
fn parse_scope_entry(entry: &Value, name: &str) -> Result<ScopeEntry, ProfileError> {
    let bad = || ProfileError::BadScopeEntry {
        name: name.to_owned(),
        entry: entry.to_string(),
    };
    let Some(text) = entry.as_str() else {
        return Err(bad());
    };
    let Some((addr_text, prefix_text)) = text.split_once('/') else {
        let network: Ipv4Addr = text.parse().map_err(|_| bad())?;
        return Ok(ScopeEntry {
            network,
            prefix: 32,
        });
    };
    let network: Ipv4Addr = addr_text.parse().map_err(|_| bad())?;
    let prefix: u8 = prefix_text.parse().map_err(|_| bad())?;
    if prefix > 32 {
        return Err(bad());
    }
    Ok(ScopeEntry { network, prefix })
}

/// Reads `field` from a profile's fields as a string, distinguishing a
/// missing field from a present non-string value.
fn string_field<'a>(
    fields: &'a Map<String, Value>,
    name: &str,
    field: &'static str,
) -> Result<&'a str, ProfileError> {
    match fields.get(field) {
        Some(Value::String(text)) => Ok(text),
        Some(_) => Err(ProfileError::NotAString {
            name: name.to_owned(),
            field,
        }),
        None => Err(ProfileError::NoField {
            name: name.to_owned(),
            field,
        }),
    }
}

/// Failures that make one `profiles` entry malformed. Every variant names
/// the offending profile — by `name` once it parses, by array index before
/// that.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// The entry is not a JSON object.
    #[error("profile at index {index} must be a JSON object")]
    NotAnObject { index: usize },
    /// The entry has no string `name` field.
    #[error("profile at index {index} must carry a string `name` field")]
    NoName { index: usize },
    /// A field required for the profile's protocol is absent.
    #[error("profile `{name}` is missing its `{field}` field")]
    NoField { name: String, field: &'static str },
    /// A required field is present but not a string.
    #[error("profile `{name}` field `{field}` must be a string")]
    NotAString { name: String, field: &'static str },
    /// The `protocol` field names no supported protocol.
    #[error("profile `{name}` names unknown protocol `{protocol}` (expected `snmp` or `api`)")]
    UnknownProtocol { name: String, protocol: String },
    /// The `scope` field is present but not an array.
    #[error("profile `{name}` scope must be an array of IPv4 addresses or CIDR entries")]
    ScopeNotArray { name: String },
    /// A scope entry is not an IPv4 address or CIDR range.
    #[error("profile `{name}` scope entry {entry} is not an IPv4 address or CIDR range")]
    BadScopeEntry { name: String, entry: String },
    /// The `discovery` field is present but not a boolean.
    #[error("profile `{name}` field `discovery` must be a boolean")]
    DiscoveryNotBool { name: String },
    /// The `snmp_version` field is present but not one of the known
    /// values (`1`, `v1`, `2c`, `v2c`).
    #[error("profile `{name}` has an unknown snmp_version `{value}` (1|v1|2c|v2c)")]
    BadSnmpVersion { name: String, value: String },
}
