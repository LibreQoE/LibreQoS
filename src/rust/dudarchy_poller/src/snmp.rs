//! Minimal SNMP v1/v2c client: single GETs for `sysDescr.0`/`sysName.0`,
//! arbitrary-OID GETs, and GETNEXT subtree walks.
//!
//! The module is layered at the seams the async migration follows: the BER
//! codec lives in [`crate::snmp_ber`], the UDP transport in
//! [`crate::snmp_transport`], and the table assembly in
//! [`crate::snmp_tables`] — all three pure and synchronous. What remains
//! here is the protocol orchestration: [`resolve_target`] resolves a vault
//! target and its referenced credential's community host-side, plus the
//! target's SNMP version; [`system_info`] sends a single request (2s
//! timeout, no retries) and decodes the reply; `get_oid` issues one GET for
//! one OID and reports an absent object distinctly; [`walk_subtree`] walks a
//! subtree with one GETNEXT per variable binding under a bounded step cap.
//! The document engine's live reader drives the same GETNEXT primitive with
//! the document's own tables; no forwarding-database collector lives here.
//! Passing [`SnmpVersion::V1`] walks v1-only
//! agents: an absent subtree ends the walk cleanly (v1 has no exception
//! values — it reports `noSuchName` as an error-status on the un-advanced
//! OID). The community never reaches output: it lives in the Debug-redacted
//! [`Community`] newtype and is named by no `Display` implementation here.
//! The orchestration is async: every request carries an explicit
//! transaction deadline, every wait is cancellable, and the walk-flavoured
//! collectors take a cancellation check that stops the walk and reports it
//! as its own outcome.

use std::fmt;
use std::time::Duration;

use serde_json::Value;

use crate::snmp_ber::{
    BerValue, REQUEST_ID, Response, SYS_DESCR, SYS_NAME, SYS_OBJECT_ID, decode_response,
    get_oid_request_bytes, get_request_bytes, next_request_bytes,
};
use crate::snmp_tables::octet_bytes;
pub use crate::snmp_tables::{
    AirStation, ArpRow, IpAddressRow, LldpNeighbour, SnmpBridgeRow, SnmpInterface, SnmpQBridgeRow,
    WalkValue,
};
use crate::snmp_transport::{exchange, exchange_retried};
use crate::vault::Vault;

/// Timeout for one SNMP GET exchange: a single request, no retries.
pub const SNMP_TIMEOUT: Duration = Duration::from_secs(2);

/// Upper bound on GETNEXT steps in one walk, so an agent that keeps
/// offering fresh OIDs cannot keep the walk running unboundedly.
const MAX_WALK_STEPS: usize = 10_000;

/// The collected SNMP FDB state for one device: Q-BRIDGE rows when a
/// document declared a `dot1q` table, BRIDGE rows when the document engine
/// decoded a `dot1d` table, and the bridge-port to ifIndex join.
///
/// The document engine builds these from a document's `fdb` probe; the core
/// keeps no collector of its own.
#[derive(Debug)]
pub struct SnmpFdb {
    /// Q-BRIDGE rows; `None` when no Q-BRIDGE observation exists — the
    /// probe was absent or failed, or the document declares no `dot1q`
    /// table. An answered-but-empty table is `Some` with no rows.
    pub qbridge: Option<Vec<SnmpQBridgeRow>>,
    /// BRIDGE-MIB rows, from a `dot1d_mac` or `dot1d_row_id` table.
    pub bridge: Option<Vec<SnmpBridgeRow>>,
    /// The bridge-port to ifIndex join, sorted by bridge port.
    pub port_if_index: Vec<(u32, u32)>,
}

/// v1 error-status value: the requested object does not exist. v1 has no
/// exception values, so this is how an absent subtree answers.
const NO_SUCH_NAME_STATUS: i64 = 2;

/// An SNMP v2c community string resolved from the vault.
///
/// `Debug` is redacted so the value cannot leak through logs, error
/// `Debug` output, or panic messages; no `Display` implementation names it.
#[derive(Clone)]
pub struct Community(String);
impl fmt::Debug for Community {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Community(***)")
    }
}

impl Community {
    /// Wraps a community resolved outside this module, for example from a
    /// vault discovery profile. Construction prints nothing; the value
    /// stays Debug-redacted.
    pub fn new(value: String) -> Self {
        Self(value)
    }
}

/// The SNMP version an exchange speaks. The vault vocabulary is `1`/`v1`
/// and `2c`/`v2c`; absent means v2c.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnmpVersion {
    /// SNMPv1: no exception values (an absent subtree answers
    /// error-status `noSuchName`), no GetBulk.
    V1,
    /// SNMPv2c: exception values (`noSuchObject`/`noSuchInstance`/
    /// `endOfMibView`) and the dialect distinctions built on them.
    V2c,
}

impl SnmpVersion {
    /// Parses the vault vocabulary; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "1" | "v1" => Some(Self::V1),
            "2c" | "v2c" => Some(Self::V2c),
            _ => None,
        }
    }

    /// The version INTEGER on the wire: v1 encodes as 0, v2c as 1.
    pub(crate) fn wire(self) -> i64 {
        match self {
            Self::V1 => 0,
            Self::V2c => 1,
        }
    }
}

/// A vault target resolved for SNMP use: its address, the community of
/// the credential its `credential` field references, and the SNMP
/// version its `snmp_version` field selects (v2c when absent).
#[derive(Debug)]
pub struct ResolvedTarget {
    /// The target's `address` string.
    pub address: String,
    /// Community string from the referenced credential.
    pub community: Community,
    /// The SNMP version the target selects.
    pub version: SnmpVersion,
}

/// Resolves `name` in `vault` to its SNMP address, community, and
/// version.
///
/// The target must exist, carry a string `address`, reference a credential
/// by name in its `credential` field, and that credential must carry a
/// string `community`. An `snmp_version` field, when present, must parse
/// in the version vocabulary (`1`/`v1`/`2c`/`v2c`). Each missing or
/// malformed piece is a distinct [`TargetError`].
pub fn resolve_target(vault: &Vault, name: &str) -> Result<ResolvedTarget, TargetError> {
    let Some(target) = vault.target(name) else {
        return Err(TargetError::UnknownTarget {
            name: name.to_owned(),
        });
    };
    let Some(address) = string_field(target, "address") else {
        return Err(TargetError::NoAddress {
            name: name.to_owned(),
        });
    };
    let Some(credential_name) = string_field(target, "credential") else {
        return Err(TargetError::NoCredentialRef {
            name: name.to_owned(),
        });
    };
    let Some(credential) = vault.credential(credential_name) else {
        return Err(TargetError::UnknownCredential {
            name: name.to_owned(),
            credential: credential_name.to_owned(),
        });
    };
    let Some(community) = string_field(credential, "community") else {
        return Err(TargetError::NoCommunity {
            credential: credential_name.to_owned(),
        });
    };
    let version = match target.get("snmp_version") {
        Some(Value::String(text)) => {
            SnmpVersion::parse(text).ok_or_else(|| TargetError::BadSnmpVersion {
                name: name.to_owned(),
                value: text.to_owned(),
            })?
        }
        Some(_) => {
            let value = target
                .get("snmp_version")
                .map(Value::to_string)
                .unwrap_or_default();
            return Err(TargetError::BadSnmpVersion {
                name: name.to_owned(),
                value,
            });
        }
        None => SnmpVersion::V2c,
    };
    Ok(ResolvedTarget {
        address: address.to_owned(),
        community: Community(community.to_owned()),
        version,
    })
}

/// System description, name, and object identifier reported by an SNMP
/// agent.
#[derive(Debug)]
pub struct SystemInfo {
    /// `sysDescr.0` as a UTF-8 string.
    pub sys_descr: String,
    /// `sysName.0` as a UTF-8 string.
    pub sys_name: String,
    /// `sysObjectID.0` as dotted text (the hardware/family OID), when the
    /// reply carries it.
    pub sys_object_id: Option<String>,
}

/// Sends one SNMP GET (v1 or v2c per `version`) for `sysDescr.0` and
/// `sysName.0` to `address` using `community`, waits up to `timeout` for
/// the single reply, and returns the decoded values.
///
/// No retries: an unanswered request is [`SnmpError::Timeout`]. The socket
/// is bound locally and connected to the agent, so only that agent's reply
/// datagram is read.
pub async fn system_info(
    address: &str,
    community: &Community,
    version: SnmpVersion,
    timeout: Duration,
) -> Result<SystemInfo, SnmpError> {
    let request = get_request_bytes(community.0.as_bytes(), version);
    let reply = exchange(address, &request, timeout).await?;
    let response = decode_response(&reply).map_err(|reason| SnmpError::Decode {
        address: address.to_owned(),
        reason: reason.to_owned(),
    })?;
    system_info_from_response(&response, community, version, address)
}

/// Validates a decoded response against `community` and the request shape,
/// then extracts `sysDescr` and `sysName`.
fn system_info_from_response(
    response: &Response<'_>,
    community: &Community,
    version: SnmpVersion,
    address: &str,
) -> Result<SystemInfo, SnmpError> {
    validate_reply(response, community, version, address)?;
    let sys_descr = string_varbind(&response.varbinds, &SYS_DESCR, "sysDescr.0", address)?;
    let sys_name = string_varbind(&response.varbinds, &SYS_NAME, "sysName.0", address)?;
    let sys_object_id = oid_varbind(&response.varbinds, &SYS_OBJECT_ID);
    Ok(SystemInfo {
        sys_descr: sys_descr.to_owned(),
        sys_name: sys_name.to_owned(),
        sys_object_id,
    })
}

/// The varbind bound to `oid` as dotted text; a missing varbind or a
/// non-OID value yields `None` (the marker is optional).
fn oid_varbind(varbinds: &[(Vec<u32>, BerValue<'_>)], oid: &[u32]) -> Option<String> {
    varbinds
        .iter()
        .find(|(bound, _)| bound.as_slice() == oid)
        .and_then(|(_, value)| match value {
            BerValue::Oid(oid) => Some(oid.clone()),
            _ => None,
        })
}

/// Checks the parts every exchange relies on: the community echo, our
/// request-id, the echoed SNMP version, and a zero error-status.
fn validate_reply(
    response: &Response<'_>,
    community: &Community,
    version: SnmpVersion,
    address: &str,
) -> Result<(), SnmpError> {
    validate_envelope(response, community, version, address)?;
    if response.error_status != 0 {
        return Err(SnmpError::ErrorStatus {
            address: address.to_owned(),
            status: response.error_status,
        });
    }
    Ok(())
}

/// Checks the parts every exchange relies on regardless of error-status:
/// the community echo, our request-id, and the echoed SNMP version.
fn validate_envelope(
    response: &Response<'_>,
    community: &Community,
    version: SnmpVersion,
    address: &str,
) -> Result<(), SnmpError> {
    let decode = |reason: &str| SnmpError::Decode {
        address: address.to_owned(),
        reason: reason.to_owned(),
    };
    if response.community != community.0.as_bytes() {
        // The agent answered, but not for this community: the credential was
        // not the one accepted. Distinct from a malformed answer, which says
        // nothing about the credential.
        return Err(SnmpError::CommunityMismatch {
            address: address.to_owned(),
        });
    }
    if response.request_id != REQUEST_ID {
        return Err(decode("reply carries another request's request-id"));
    }
    if response.version != version.wire() {
        return Err(decode("reply carries a different SNMP version"));
    }
    Ok(())
}

/// Finds the varbind bound to `oid` and returns its value as UTF-8. A
/// missing varbind or invalid UTF-8 is a decode failure; a non-string value
/// (including v2c exceptions such as `noSuchInstance`) is
/// [`SnmpError::UnsupportedValue`].
fn string_varbind<'a>(
    varbinds: &'a [(Vec<u32>, BerValue<'a>)],
    oid: &[u32],
    name: &str,
    address: &str,
) -> Result<&'a str, SnmpError> {
    let decode = |reason: String| SnmpError::Decode {
        address: address.to_owned(),
        reason,
    };
    let Some((_, value)) = varbinds.iter().find(|(bound, _)| bound.as_slice() == oid) else {
        return Err(decode(format!("reply is missing {name}")));
    };
    match value {
        BerValue::OctetString(bytes) => {
            std::str::from_utf8(bytes).map_err(|_| decode(format!("{name} is not valid UTF-8")))
        }
        _ => Err(SnmpError::UnsupportedValue {
            address: address.to_owned(),
        }),
    }
}

/// One walked variable binding: the responding OID and its value when the
/// agent answered with an OCTET STRING. Other value types (integers,
/// counters) and v2c exceptions such as `noSuchObject` yield `None`, so a
/// sparse table column simply contributes no value.
pub type WalkedVarbind = (Vec<u32>, Option<Vec<u8>>);

/// The outcome of one bounded subtree walk: the complete set of bindings
/// below the root, or — when the caller's cancellation check fired
/// mid-walk — the bindings collected so far, distinctly marked.
#[derive(Debug)]
pub enum WalkOutcome {
    /// Every binding below the root, in lexicographic order.
    Complete(Vec<WalkedVarbind>),
    /// Cancellation stopped the walk; the bindings collected so far.
    Cancelled(Vec<WalkedVarbind>),
}

/// Walks the MIB subtree rooted at `root` with one GETNEXT per step and
/// returns every binding below `root` in lexicographic order.
///
/// Each step is one request/reply exchange with the same no-retry timeout
/// as [`system_info`]. The walk ends cleanly when the agent reports
/// `endOfMibView`, a reply leaves the subtree, or — on v1 — the agent
/// answers a step with error-status `noSuchName`; it fails on malformed
/// replies, other error statuses, an OID that does not advance, or when
/// it reaches the bounded step cap ([`SnmpError::WalkCapExceeded`]).
/// `cancel` is consulted before every step; when it fires the walk stops
/// and reports [`WalkOutcome::Cancelled`] carrying the bindings collected
/// so far.
pub async fn walk_subtree(
    address: &str,
    community: &Community,
    root: &[u32],
    version: SnmpVersion,
    timeout: Duration,
    cancel: &(dyn Fn() -> bool + Send + Sync),
) -> Result<WalkOutcome, SnmpError> {
    let (rows, complete) =
        match walk_values(address, community, root, version, timeout, cancel).await? {
            Walked::Rows(rows) => (rows, true),
            Walked::Cancelled(rows) => (rows, false),
            Walked::NoSuchObject => return Ok(WalkOutcome::Complete(Vec::new())),
        };
    let walked = rows
        .into_iter()
        .map(|(oid, value)| (oid, octet_bytes(value)))
        .collect();
    Ok(if complete {
        WalkOutcome::Complete(walked)
    } else {
        WalkOutcome::Cancelled(walked)
    })
}

/// Walks the MIB subtree rooted at `root` with one GETNEXT per step and
/// returns every binding below `root` in lexicographic order, keeping
/// integer values that [`walk_subtree`] drops.
///
/// Each step is one request/reply exchange with the same no-retry timeout
/// as [`system_info`]. The walk ends cleanly when the agent reports
/// `endOfMibView` or a reply leaves the subtree; it fails on malformed
/// replies, an OID that does not advance, or when it reaches the bounded
/// step cap ([`SnmpError::WalkCapExceeded`]).
/// The outcome of one subtree walk: the rows below the root, or the
/// agent's `noSuchObject` answer for the root itself — an absent table,
/// which is not the same as an empty one.
#[derive(Debug)]
pub(crate) enum Walked {
    /// Every binding below the root, in lexicographic order.
    Rows(Vec<(Vec<u32>, WalkValue)>),
    /// The agent answered the root itself with `noSuchObject` (or
    /// `noSuchInstance`): the object does not exist.
    NoSuchObject,
    /// The caller's cancellation check fired between steps: the bindings
    /// collected so far, in walk order.
    Cancelled(Vec<(Vec<u32>, WalkValue)>),
}

/// Walks the MIB subtree rooted at `root` with one GETNEXT per step and
/// returns every binding below `root` in lexicographic order, keeping
/// integer and counter values that [`walk_subtree`] drops. Each step
/// retries once on a lost datagram.
///
/// The walk ends cleanly when the agent reports `endOfMibView`, a reply
/// leaves the subtree, or — on [`SnmpVersion::V1`], which has no
/// exception values — the agent answers a step with error-status
/// `noSuchName` (an absent or exhausted subtree surfaces as the rows
/// collected so far, possibly none). It reports [`Walked::NoSuchObject`]
/// when a v2c agent declines the root itself, and fails on malformed
/// replies, other error statuses, an OID that does not advance (short of
/// the exception above), or when it reaches the bounded step cap
/// ([`SnmpError::WalkCapExceeded`]).
pub(crate) async fn walk_values(
    address: &str,
    community: &Community,
    root: &[u32],
    version: SnmpVersion,
    timeout: Duration,
    cancel: &(dyn Fn() -> bool + Send + Sync),
) -> Result<Walked, SnmpError> {
    let mut walked: Vec<(Vec<u32>, WalkValue)> = Vec::new();
    let mut cursor = root.to_vec();
    loop {
        if cancel() {
            return Ok(Walked::Cancelled(walked));
        }
        if walked.len() >= MAX_WALK_STEPS {
            return Err(SnmpError::WalkCapExceeded {
                address: address.to_owned(),
                steps: MAX_WALK_STEPS,
            });
        }
        let request = next_request_bytes(community.0.as_bytes(), version, &cursor);
        let reply = exchange_retried(address, &request, timeout).await?;
        let response = decode_response(&reply).map_err(|reason| SnmpError::Decode {
            address: address.to_owned(),
            reason: reason.to_owned(),
        })?;
        validate_envelope(&response, community, version, address)?;
        // v1 has no exception values: an absent or exhausted subtree
        // answers with error-status `noSuchName` on the un-advanced OID.
        // That is a clean end; the rows collected so far stand. Any
        // other non-zero status keeps failing as today.
        if response.error_status != 0 {
            if version == SnmpVersion::V1 && response.error_status == NO_SUCH_NAME_STATUS {
                return Ok(Walked::Rows(walked));
            }
            return Err(SnmpError::ErrorStatus {
                address: address.to_owned(),
                status: response.error_status,
            });
        }
        let Some((oid, value)) = response.varbinds.first() else {
            return Err(SnmpError::Decode {
                address: address.to_owned(),
                reason: "walk reply carries no varbinds".to_owned(),
            });
        };
        if matches!(value, BerValue::EndOfMibView) {
            break;
        }
        if !oid.starts_with(root) {
            break;
        }
        if *oid <= cursor {
            // An exception bound to the non-advanced OID is the agent
            // saying the requested object does not exist — an absent
            // table, not a broken walk.
            if matches!(value, BerValue::NoSuchObject | BerValue::NoSuchInstance) {
                return Ok(Walked::NoSuchObject);
            }
            return Err(SnmpError::Decode {
                address: address.to_owned(),
                reason: "agent did not advance the oid".to_owned(),
            });
        }
        walked.push((oid.clone(), walk_value(value)));
        cursor = oid.clone();
    }
    Ok(Walked::Rows(walked))
}

/// A GET that cannot be issued, as distinct from one the agent failed to
/// answer. Nothing was sent for the former, so it is not a transport error;
/// the engine's reader reports both as `Error`, with different reasons.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GetError {
    /// A one-arc OID has no BER encoding: the first two arcs share one
    /// subidentifier.
    #[error("cannot request OID `{oid}`: an SNMP OID needs at least two arcs")]
    ShortOid {
        /// The rejected OID as dotted text.
        oid: String,
    },
    /// The exchange itself failed.
    #[error(transparent)]
    Snmp(#[from] SnmpError),
}

/// One SNMP GET for a single OID: the answered binding, or `None` when the
/// object does not exist (a v2c `noSuchObject`/`noSuchInstance` exception,
/// or a v1 `noSuchName` error-status).
///
/// One request, no retries: an unanswered request is
/// [`SnmpError::Timeout`]. The reply must echo our community, request-id,
/// and version, and bind the OID the request named.
pub(crate) async fn get_oid(
    address: &str,
    community: &Community,
    version: SnmpVersion,
    oid: &[u32],
    timeout: Duration,
) -> Result<Option<(Vec<u32>, WalkValue)>, GetError> {
    // The first two arcs are packed into one BER subidentifier; a shorter
    // OID has no encoding at all.
    if oid.len() < 2 {
        return Err(GetError::ShortOid {
            oid: oid.iter().map(u32::to_string).collect::<Vec<_>>().join("."),
        });
    }
    let request = get_oid_request_bytes(community.0.as_bytes(), version, oid);
    let reply = exchange(address, &request, timeout).await?;
    let response = decode_response(&reply).map_err(|reason| SnmpError::Decode {
        address: address.to_owned(),
        reason: reason.to_owned(),
    })?;
    validate_envelope(&response, community, version, address)?;
    if response.error_status != 0 {
        // v1 has no exception values: a missing object answers with
        // error-status `noSuchName`.
        if version == SnmpVersion::V1 && response.error_status == NO_SUCH_NAME_STATUS {
            return Ok(None);
        }
        return Err(SnmpError::ErrorStatus {
            address: address.to_owned(),
            status: response.error_status,
        }
        .into());
    }
    let Some((bound, value)) = response.varbinds.first() else {
        return Err(SnmpError::Decode {
            address: address.to_owned(),
            reason: "get reply carries no varbinds".to_owned(),
        }
        .into());
    };
    if bound.as_slice() != oid {
        return Err(SnmpError::Decode {
            address: address.to_owned(),
            reason: "reply carries another object's oid".to_owned(),
        }
        .into());
    }
    match value {
        BerValue::NoSuchObject | BerValue::NoSuchInstance | BerValue::EndOfMibView => Ok(None),
        value => Ok(Some((bound.clone(), walk_value(value)))),
    }
}

/// The walked BER value as an owned [`WalkValue`].
fn walk_value(value: &BerValue<'_>) -> WalkValue {
    match value {
        BerValue::OctetString(bytes) => WalkValue::Bytes(bytes.to_vec()),
        BerValue::Integer(value) => WalkValue::Integer(*value),
        BerValue::Counter64(value) => WalkValue::Counter64(*value),
        BerValue::TimeTicks(value) => WalkValue::TimeTicks(*value),
        BerValue::IpAddress(text) => WalkValue::IpAddress(text.clone()),
        BerValue::Oid(text) => WalkValue::Oid(text.clone()),
        _ => WalkValue::Empty,
    }
}

/// The string value of `key` on a free-form vault entry, if it is a string.
fn string_field<'a>(entry: &'a Value, key: &'static str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}

/// Failures resolving a vault target for SNMP.
#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    /// No target entry carries this name.
    #[error("unknown target `{name}` in vault")]
    UnknownTarget { name: String },
    /// The target exists but has no string `address` field.
    #[error("target `{name}` has no `address`")]
    NoAddress { name: String },
    /// The target exists but has no string `credential` field.
    #[error("target `{name}` has no `credential` reference")]
    NoCredentialRef { name: String },
    /// The target's `credential` names no credential in the vault.
    #[error("target `{name}` references unknown credential `{credential}`")]
    UnknownCredential { name: String, credential: String },
    /// The referenced credential has no string `community` field.
    #[error("credential `{credential}` has no `community`")]
    NoCommunity { credential: String },
    /// The target's `snmp_version` field is present but not one of the
    /// known values (`1`, `v1`, `2c`, `v2c`).
    #[error("target `{name}` has an unknown snmp_version `{value}` (1|v1|2c|v2c)")]
    BadSnmpVersion { name: String, value: String },
}

/// Failures of one SNMP GET exchange.
#[derive(Debug, thiserror::Error)]
pub enum SnmpError {
    /// A socket or resolver operation failed.
    #[error("SNMP exchange with {address} failed")]
    Io {
        address: String,
        #[source]
        source: std::io::Error,
    },
    /// The agent did not answer within the timeout.
    #[error("no SNMP reply from {address} within {timeout:?}")]
    Timeout { address: String, timeout: Duration },
    /// The reply was malformed or did not match the request.
    #[error("malformed SNMP response from {address}: {reason}")]
    Decode { address: String, reason: String },
    /// The agent answered with a community other than the one we presented:
    /// the agent knows another credential, not ours.
    #[error("SNMP agent at {address} answered for another community")]
    CommunityMismatch { address: String },
    /// The agent reported a non-zero error-status.
    #[error("SNMP agent at {address} reported error-status {status}")]
    ErrorStatus { address: String, status: i64 },
    /// A requested value is present but not an OCTET STRING.
    #[error("sysDescr/sysName from {address} are not OCTET STRING values")]
    UnsupportedValue { address: String },
    /// The walk reached its step cap before the agent ended the subtree.
    #[error("SNMP walk of {address} hit the {steps}-step cap")]
    WalkCapExceeded { address: String, steps: usize },
    /// The caller cancelled the walk (distinct from a timeout);
    /// [`walk_subtree`] reports partial walks through [`WalkOutcome`]
    /// instead.
    #[error("SNMP walk of {address} was cancelled")]
    Cancelled { address: String },
}

#[cfg(test)]
mod tests {
    /// An OBJECT IDENTIFIER varbind decodes to dotted text, not `Empty`:
    /// the document engine's `oid` parser depends on it.
    #[test]
    fn an_oid_value_decodes_to_dotted_text() {
        let value = walk_value(&BerValue::Oid("1.3.6.1.4.1.10002.1".to_owned()));
        assert_eq!(value, WalkValue::Oid("1.3.6.1.4.1.10002.1".to_owned()));
    }

    /// A one-arc OID cannot be encoded: the first two arcs share one
    /// subidentifier. The rejection is named and happens before any socket.
    #[tokio::test]
    async fn a_one_arc_oid_is_rejected_before_any_exchange() {
        let community = Community::new("public".to_owned());
        let error = get_oid(
            "127.0.0.1:9",
            &community,
            SnmpVersion::V2c,
            &[1],
            Duration::from_millis(1),
        )
        .await
        .expect_err("a one-arc OID is not encodable");
        match error {
            GetError::ShortOid { oid } => assert_eq!(oid, "1"),
            other => panic!("expected ShortOid, got {other:?}"),
        }
    }

    use super::*;
    use crate::snmp_ber::{
        GET_PUBLIC, RESPONSE_GEN_ERR, RESPONSE_NO_SUCH_INSTANCE, RESPONSE_PUBLIC,
    };

    /// A vault fixture covering the happy resolution path.
    const FULL_VAULT: &str = r#"{
        "targets": [{"name": "t", "address": "10.0.0.1", "credential": "c"}],
        "credentials": [{"name": "c", "community": "public"}]
    }"#;

    #[test]
    fn responses_of_a_foreign_version_are_rejected() {
        let mut wrong = RESPONSE_PUBLIC;
        wrong[4] = 0x00; // claims v1 while the exchange asked v2c
        let error = system_info_from_response(
            &decode_response(&wrong).unwrap(),
            &Community("public".to_owned()),
            SnmpVersion::V2c,
            "a",
        )
        .unwrap_err();
        let SnmpError::Decode { reason, .. } = error else {
            panic!("expected Decode");
        };
        assert_eq!(reason, "reply carries a different SNMP version");
    }

    #[test]
    fn resolve_target_reads_the_snmp_version_vocabulary() {
        let vault = Vault::parse(
            r#"{"targets": [
                    {"name": "one", "address": "10.0.0.1", "credential": "c", "snmp_version": "1"},
                    {"name": "vee", "address": "10.0.0.2", "credential": "c", "snmp_version": "v2c"},
                    {"name": "default", "address": "10.0.0.3", "credential": "c"},
                    {"name": "bad", "address": "10.0.0.4", "credential": "c", "snmp_version": "3"}
                ],
                "credentials": [{"name": "c", "community": "public"}]}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_target(&vault, "one").unwrap().version,
            SnmpVersion::V1
        );
        assert_eq!(
            resolve_target(&vault, "vee").unwrap().version,
            SnmpVersion::V2c
        );
        assert_eq!(
            resolve_target(&vault, "default").unwrap().version,
            SnmpVersion::V2c,
            "absent means v2c"
        );
        match resolve_target(&vault, "bad") {
            Err(TargetError::BadSnmpVersion { value, .. }) => assert_eq!(value, "3"),
            other => panic!("expected BadSnmpVersion, got {other:?}"),
        }
    }

    #[test]
    fn response_parses_expected_values() {
        let response = decode_response(&RESPONSE_PUBLIC).expect("fixture decodes");
        assert_eq!(response.community, b"public");
        assert_eq!(response.request_id, 1);
        assert_eq!(response.error_status, 0);
        assert_eq!(response.varbinds.len(), 3);
        let sys_info = system_info_from_response(
            &decode_response(&RESPONSE_PUBLIC).expect("fixture decodes"),
            &Community("public".to_owned()),
            SnmpVersion::V2c,
            "a",
        )
        .unwrap();
        assert_eq!(sys_info.sys_descr, "RouterOS CCR1009-8G-1S");
        assert_eq!(
            sys_info.sys_object_id.as_deref(),
            Some("1.3.6.1.4.1.14988.1")
        );
        assert_eq!(sys_info.sys_name, "FSW-Office-Core");
    }

    #[test]
    fn error_status_is_decoded() {
        let response = decode_response(&RESPONSE_GEN_ERR).expect("fixture decodes");
        assert_eq!(response.error_status, 5);
        let error = system_info_from_response(
            &response,
            &Community("public".to_owned()),
            SnmpVersion::V2c,
            "a",
        )
        .unwrap_err();
        assert!(matches!(error, SnmpError::ErrorStatus { status: 5, .. }));
    }

    #[test]
    fn mismatched_community_is_rejected() {
        let response = decode_response(&RESPONSE_PUBLIC).expect("fixture decodes");
        let error = system_info_from_response(
            &response,
            &Community("private".to_owned()),
            SnmpVersion::V2c,
            "a",
        )
        .unwrap_err();
        assert!(matches!(error, SnmpError::CommunityMismatch { .. }));
    }

    #[test]
    fn foreign_request_id_is_rejected() {
        let mut other = RESPONSE_PUBLIC;
        other[17] = 0x02;
        let response = decode_response(&other).unwrap();
        let error = system_info_from_response(
            &response,
            &Community("public".to_owned()),
            SnmpVersion::V2c,
            "a",
        )
        .unwrap_err();
        assert!(matches!(error, SnmpError::Decode { .. }));
    }

    #[test]
    fn no_such_instance_is_an_unsupported_value() {
        let response = decode_response(&RESPONSE_NO_SUCH_INSTANCE).expect("fixture decodes");
        let error = system_info_from_response(
            &response,
            &Community("public".to_owned()),
            SnmpVersion::V2c,
            "a",
        )
        .unwrap_err();
        assert!(matches!(error, SnmpError::UnsupportedValue { .. }));
    }

    #[test]
    fn missing_sys_descr_is_reported_by_name() {
        let mut missing = RESPONSE_PUBLIC;
        missing[36] = 0x05; // first varbind's OID becomes sysName.0 too
        let response = decode_response(&missing).unwrap();
        let error = system_info_from_response(
            &response,
            &Community("public".to_owned()),
            SnmpVersion::V2c,
            "a",
        )
        .unwrap_err();
        let SnmpError::Decode { reason, .. } = error else {
            panic!("expected SnmpError::Decode");
        };
        assert_eq!(reason, "reply is missing sysDescr.0");
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let mut invalid = RESPONSE_PUBLIC;
        invalid[40] = 0xff; // first byte of the sysDescr string
        let response = decode_response(&invalid).unwrap();
        let error = system_info_from_response(
            &response,
            &Community("public".to_owned()),
            SnmpVersion::V2c,
            "a",
        )
        .unwrap_err();
        let SnmpError::Decode { reason, .. } = error else {
            panic!("expected SnmpError::Decode");
        };
        assert_eq!(reason, "sysDescr.0 is not valid UTF-8");
    }

    #[test]
    fn resolution_happy_path_carries_address_and_community() {
        let vault = Vault::parse(FULL_VAULT).unwrap();
        let resolved = resolve_target(&vault, "t").unwrap();
        assert_eq!(resolved.address, "10.0.0.1");
        // The community feeds the request encoding: the fixture request for
        // "public" proves the resolved value arrived intact.
        assert_eq!(
            get_request_bytes(resolved.community.0.as_bytes(), SnmpVersion::V2c),
            GET_PUBLIC.to_vec()
        );
    }

    #[test]
    fn resolved_target_debug_redacts_community() {
        let vault = Vault::parse(FULL_VAULT).unwrap();
        let resolved = resolve_target(&vault, "t").unwrap();
        let debug = format!("{resolved:?}");
        assert!(debug.contains("10.0.0.1"), "address stays visible: {debug}");
        assert!(
            !debug.contains("public"),
            "community must be redacted: {debug}"
        );
    }

    #[test]
    fn resolution_branches_report_distinct_errors() {
        let vault = Vault::parse(FULL_VAULT).unwrap();
        match resolve_target(&vault, "no-such-target") {
            Err(TargetError::UnknownTarget { name }) => assert_eq!(name, "no-such-target"),
            other => panic!("expected UnknownTarget, got {other:?}"),
        }

        let no_address =
            Vault::parse(r#"{"targets": [{"name": "t", "credential": "c"}]}"#).unwrap();
        match resolve_target(&no_address, "t") {
            Err(TargetError::NoAddress { name }) => assert_eq!(name, "t"),
            other => panic!("expected NoAddress, got {other:?}"),
        }

        let no_ref =
            Vault::parse(r#"{"targets": [{"name": "t", "address": "10.0.0.1"}]}"#).unwrap();
        match resolve_target(&no_ref, "t") {
            Err(TargetError::NoCredentialRef { name }) => assert_eq!(name, "t"),
            other => panic!("expected NoCredentialRef, got {other:?}"),
        }

        let unknown_ref = Vault::parse(
            r#"{"targets": [{"name": "t", "address": "10.0.0.1", "credential": "x"}]}"#,
        )
        .unwrap();
        match resolve_target(&unknown_ref, "t") {
            Err(TargetError::UnknownCredential { credential, .. }) => {
                assert_eq!(credential, "x");
            }
            other => panic!("expected UnknownCredential, got {other:?}"),
        }

        let no_community = Vault::parse(
            r#"{"targets": [{"name": "t", "address": "10.0.0.1", "credential": "c"}],
                "credentials": [{"name": "c"}]}"#,
        )
        .unwrap();
        match resolve_target(&no_community, "t") {
            Err(TargetError::NoCommunity { credential }) => assert_eq!(credential, "c"),
            other => panic!("expected NoCommunity, got {other:?}"),
        }
    }
}
