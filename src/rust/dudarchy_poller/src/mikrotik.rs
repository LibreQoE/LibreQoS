//! The RouterOS read-only API client: vault resolution, the generic named
//! command execution, and its typed errors.
//!
//! The module is layered at the seams the async migration follows: word
//! framing, sentence parsing, and the `=key=value` conventions live in
//! [`crate::mikrotik_proto`]; the TCP connection, login, and framed
//! sentence exchange live in [`crate::mikrotik_transport`]. What remains
//! here is the public API and the vault resolution:
//! [`resolve_api_target`] resolves a vault target and the username and
//! password of the credential its `api_credential` field references
//! host-side; [`command`] connects, logs in, and returns one raw
//! key/value row per `!re` sentence of a core-registered read-only print
//! command. The command name → protocol path allow-list is core; the
//! adapter document chooses which commands run and maps their fields.
//!
//! Every exchange is async: each I/O step carries its own deadline, and
//! the caller's cancellation check stops the exchange as its own error —
//! the connection is then dropped, never reused. The password never
//! reaches output: it lives in the Debug-redacted [`Password`] newtype and
//! is named by no `Display` implementation here.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use serde_json::Value;

use crate::commands::{CommandName, CommandRow};
use crate::mikrotik_transport::{MAX_PRINT_ROWS, connect, login, request_sentences};

/// The cancellation check every RouterOS exchange consults: before each
/// login write, between the login reply phases, and before every reply
/// sentence.
type Cancel<'a> = &'a (dyn Fn() -> bool + Send + Sync);
use crate::vault::Vault;

pub use crate::mikrotik_proto::{
    ApiNeighbour, RouterOsAddress, RouterOsArp, RouterOsBridgeHost, RouterOsBridgePort,
    RouterOsBridgeVlan, RouterOsInterface, RouterOsNatRule,
};
pub use crate::mikrotik_transport::MAX_NEIGHBOUR_ROWS;

/// Timeout for one MikroTik API exchange: connect, login, and the command
/// query each share this bound, with no retries.
pub const MIKROTIK_TIMEOUT: Duration = Duration::from_secs(2);

/// A RouterOS API password resolved from the vault.
///
/// `Debug` is redacted so the value cannot leak through logs, error
/// `Debug` output, or panic messages; no `Display` implementation names it.
pub struct Password(pub(crate) String);

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(***)")
    }
}

impl Password {
    /// Wraps a password resolved outside this module, for example from a
    /// vault discovery profile. Construction prints nothing; the value
    /// stays Debug-redacted.
    pub fn new(value: String) -> Self {
        Self(value)
    }
}

/// A vault target resolved for the RouterOS API: its address and the
/// username and password of the credential its `api_credential` field
/// references.
#[derive(Debug)]
pub struct ApiTarget {
    /// The target's `address` string.
    pub address: String,
    /// Username from the referenced credential.
    pub username: String,
    /// Password from the referenced credential, redacted in `Debug`.
    pub password: Password,
}

/// Resolves `name` in `vault` to its API address and api credential.
///
/// The target must exist, carry a string `address`, reference a credential
/// by name in its `api_credential` field, and that credential must carry
/// string `username` and `password` fields. Each missing piece is a
/// distinct [`ApiTargetError`].
pub fn resolve_api_target(vault: &Vault, name: &str) -> Result<ApiTarget, ApiTargetError> {
    let Some(target) = vault.target(name) else {
        return Err(ApiTargetError::UnknownTarget {
            name: name.to_owned(),
        });
    };
    let Some(address) = string_field(target, "address") else {
        return Err(ApiTargetError::NoAddress {
            name: name.to_owned(),
        });
    };
    let Some(credential_name) = string_field(target, "api_credential") else {
        return Err(ApiTargetError::NoApiCredentialRef {
            name: name.to_owned(),
        });
    };
    let Some(credential) = vault.credential(credential_name) else {
        return Err(ApiTargetError::UnknownCredential {
            name: name.to_owned(),
            credential: credential_name.to_owned(),
        });
    };
    let Some(username) = string_field(credential, "username") else {
        return Err(ApiTargetError::NoUsername {
            credential: credential_name.to_owned(),
        });
    };
    let Some(password) = string_field(credential, "password") else {
        return Err(ApiTargetError::NoPassword {
            credential: credential_name.to_owned(),
        });
    };
    Ok(ApiTarget {
        address: address.to_owned(),
        username: username.to_owned(),
        password: Password(password.to_owned()),
    })
}

/// Logs in to the RouterOS API at `address` with `username` and `password`
/// and runs one core-registered read-only print command, returning one
/// key/value row per `!re` sentence. A key the router sent no value for is
/// absent from the row, so absent and empty values are alike unknown.
///
/// The whole exchange logs in with the modern plain form and, when the
/// router answers with a legacy `=ret=` challenge (pre-6.43 firmware),
/// follows the bounded challenge-response continuation. Every I/O step is
/// bounded by `timeout`, with no retries.
pub async fn command(
    address: &str,
    username: &str,
    password: &Password,
    name: CommandName,
    timeout: Duration,
    cancel: Cancel<'_>,
) -> Result<Vec<CommandRow>, MikrotikError> {
    let mut stream = connect(address, timeout).await?;
    login(&mut stream, address, username, password, timeout, cancel).await?;
    let sentences = request_sentences(
        &mut stream,
        address,
        command_path(name),
        row_cap(name),
        timeout,
        cancel,
    )
    .await?;
    Ok(sentences.iter().map(|sentence| row_of(sentence)).collect())
}

/// The closed command-name → protocol-path allow-list. No other command is
/// reachable through this client.
fn command_path(name: CommandName) -> &'static str {
    match name {
        CommandName::SystemIdentity => "/system/identity/print",
        CommandName::SystemRouterboard => "/system/routerboard/print",
        CommandName::IpNeighbor => "/ip/neighbor/print",
        CommandName::Interfaces => "/interface/print",
        CommandName::IpAddress => "/ip/address/print",
        CommandName::IpArp => "/ip/arp/print",
        CommandName::BridgePort => "/interface/bridge/port/print",
        CommandName::BridgeVlan => "/interface/bridge/vlan/print",
        CommandName::BridgeHost => "/interface/bridge/host/print",
        CommandName::SystemResource => "/system/resource/print",
        CommandName::SystemHealth => "/system/health/print",
        CommandName::IpFirewallNat => "/ip/firewall/nat/print",
        CommandName::IpDhcpServerLease => "/ip/dhcp-server/lease/print",
    }
}

/// The row cap for one command's reply. The neighbour table is one `!re`
/// sentence per row and legitimately exceeds the fixed print cap.
fn row_cap(name: CommandName) -> usize {
    match name {
        CommandName::IpNeighbor => MAX_NEIGHBOUR_ROWS,
        _ => MAX_PRINT_ROWS,
    }
}

/// One `!re` sentence as a row: every non-empty `=key=value` word, in
/// reply order; the `!re` marker itself carries no key.
fn row_of(sentence: &[String]) -> CommandRow {
    let mut row = BTreeMap::new();
    for word in sentence {
        let Some(word) = word.strip_prefix('=') else {
            continue;
        };
        let Some((key, value)) = word.split_once('=') else {
            continue;
        };
        if !value.is_empty() {
            row.insert(key.to_owned(), value.to_owned());
        }
    }
    row
}

/// The string value of `key` on a free-form vault entry, if it is a string.
fn string_field<'a>(entry: &'a Value, key: &'static str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}

/// Failures resolving a vault target for the RouterOS API.
#[derive(Debug, thiserror::Error)]
pub enum ApiTargetError {
    /// No target entry carries this name.
    #[error("unknown target `{name}` in vault")]
    UnknownTarget { name: String },
    /// The target exists but has no string `address` field.
    #[error("target `{name}` has no `address`")]
    NoAddress { name: String },
    /// The target exists but has no string `api_credential` field.
    #[error("target `{name}` has no `api_credential` reference")]
    NoApiCredentialRef { name: String },
    /// The target's `api_credential` names no credential in the vault.
    #[error("target `{name}` references unknown credential `{credential}`")]
    UnknownCredential { name: String, credential: String },
    /// The referenced credential has no string `username` field.
    #[error("credential `{credential}` has no `username`")]
    NoUsername { credential: String },
    /// The referenced credential has no string `password` field.
    #[error("credential `{credential}` has no `password`")]
    NoPassword { credential: String },
}

/// Failures of one RouterOS API exchange.
#[derive(Debug, thiserror::Error)]
pub enum MikrotikError {
    /// A socket, resolver, or connection operation failed.
    #[error("MikroTik API exchange with {address} failed")]
    Io {
        address: String,
        #[source]
        source: std::io::Error,
    },
    /// The exchange violated the framing or login/reply protocol.
    #[error("malformed MikroTik API exchange with {address}: {reason}")]
    Protocol { address: String, reason: String },
    /// The router rejected the login with a `!trap`/`!fatal` reply.
    #[error("login to {address} failed: {message}")]
    LoginFailed { address: String, message: String },
    /// The router rejected the command with a `!trap`/`!fatal` reply.
    #[error("command on {address} failed: {message}")]
    CommandFailed { address: String, message: String },
    /// The caller cancelled the exchange; the connection was discarded
    /// and the next request reconnects from scratch.
    #[error("RouterOS API exchange with {address} was cancelled")]
    Cancelled { address: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_debug_is_redacted() {
        let password = Password("hunter2".to_owned());
        let debug = format!("{password:?}");
        assert_eq!(debug, "Password(***)");
    }

    #[test]
    fn resolution_happy_path_carries_address_and_credentials() {
        let vault = Vault::parse(FULL_VAULT).unwrap();
        let resolved = resolve_api_target(&vault, "t").unwrap();
        assert_eq!(resolved.address, "10.0.0.1");
        assert_eq!(resolved.username, "admin");
        // The password feeds the login sentence: encoding `=password=op3n`
        // proves the resolved value arrived intact.
        let mut out = Vec::new();
        crate::mikrotik_proto::encode_word(&mut out, &format!("=password={}", resolved.password.0))
            .expect("login word should encode");
        assert_eq!(out, crate::mikrotik_proto::encoded("=password=op3n"));
    }

    #[test]
    fn resolved_target_debug_redacts_password() {
        let vault = Vault::parse(FULL_VAULT).unwrap();
        let resolved = resolve_api_target(&vault, "t").unwrap();
        let debug = format!("{resolved:?}");
        assert!(debug.contains("10.0.0.1"), "address stays visible: {debug}");
        assert!(debug.contains("admin"), "username stays visible: {debug}");
        assert!(
            !debug.contains("op3n"),
            "password must be redacted: {debug}"
        );
    }

    #[test]
    fn resolution_branches_report_distinct_errors() {
        let vault = Vault::parse(FULL_VAULT).unwrap();
        match resolve_api_target(&vault, "no-such-target") {
            Err(ApiTargetError::UnknownTarget { name }) => assert_eq!(name, "no-such-target"),
            other => panic!("expected UnknownTarget, got {other:?}"),
        }

        let no_address =
            Vault::parse(r#"{"targets": [{"name": "t", "api_credential": "c"}]}"#).unwrap();
        match resolve_api_target(&no_address, "t") {
            Err(ApiTargetError::NoAddress { name }) => assert_eq!(name, "t"),
            other => panic!("expected NoAddress, got {other:?}"),
        }

        let no_ref =
            Vault::parse(r#"{"targets": [{"name": "t", "address": "10.0.0.1"}]}"#).unwrap();
        match resolve_api_target(&no_ref, "t") {
            Err(ApiTargetError::NoApiCredentialRef { name }) => assert_eq!(name, "t"),
            other => panic!("expected NoApiCredentialRef, got {other:?}"),
        }

        let unknown_ref = Vault::parse(
            r#"{"targets": [{"name": "t", "address": "10.0.0.1", "api_credential": "x"}]}"#,
        )
        .unwrap();
        match resolve_api_target(&unknown_ref, "t") {
            Err(ApiTargetError::UnknownCredential { credential, .. }) => {
                assert_eq!(credential, "x");
            }
            other => panic!("expected UnknownCredential, got {other:?}"),
        }

        let no_username = Vault::parse(
            r#"{"targets": [{"name": "t", "address": "10.0.0.1", "api_credential": "c"}],
                "credentials": [{"name": "c"}]}"#,
        )
        .unwrap();
        match resolve_api_target(&no_username, "t") {
            Err(ApiTargetError::NoUsername { credential }) => assert_eq!(credential, "c"),
            other => panic!("expected NoUsername, got {other:?}"),
        }

        let no_password = Vault::parse(
            r#"{"targets": [{"name": "t", "address": "10.0.0.1", "api_credential": "c"}],
                "credentials": [{"name": "c", "username": "admin"}]}"#,
        )
        .unwrap();
        match resolve_api_target(&no_password, "t") {
            Err(ApiTargetError::NoPassword { credential }) => assert_eq!(credential, "c"),
            other => panic!("expected NoPassword, got {other:?}"),
        }
    }

    #[test]
    fn a_reply_row_keeps_every_non_empty_value() {
        let sentence = vec![
            "!re".to_owned(),
            "=.id=*1".to_owned(),
            "=name=ether1".to_owned(),
            "=comment=".to_owned(),
            "=type=ether".to_owned(),
        ];
        let row = row_of(&sentence);
        assert_eq!(row.get(".id").map(String::as_str), Some("*1"));
        assert_eq!(row.get("name").map(String::as_str), Some("ether1"));
        assert_eq!(row.get("type").map(String::as_str), Some("ether"));
        assert_eq!(row.get("comment"), None, "an empty value is unknown");
    }

    #[test]
    fn every_command_name_has_one_print_path() {
        let mut paths = Vec::new();
        for name in [
            CommandName::SystemIdentity,
            CommandName::SystemRouterboard,
            CommandName::IpNeighbor,
            CommandName::Interfaces,
            CommandName::IpAddress,
            CommandName::IpArp,
            CommandName::BridgePort,
            CommandName::BridgeVlan,
            CommandName::BridgeHost,
            CommandName::SystemResource,
            CommandName::SystemHealth,
            CommandName::IpFirewallNat,
            CommandName::IpDhcpServerLease,
        ] {
            let path = command_path(name);
            assert!(path.starts_with('/') && path.ends_with("/print"), "{path}");
            paths.push(path);
        }
        paths.sort_unstable();
        let count = paths.len();
        paths.dedup();
        assert_eq!(paths.len(), count, "one path per command");
    }

    /// A vault fixture covering the happy resolution path.
    const FULL_VAULT: &str = r#"{
        "targets": [{"name": "t", "address": "10.0.0.1", "api_credential": "c"}],
        "credentials": [{"name": "c", "username": "admin", "password": "op3n"}]
    }"#;
}
