//! `x-bearer` authentication for the mounted `/api/v3` surface.
//!
//! The node manager already mints local API keys ([`super::local_api_keys`]):
//! `lqos_api_<uuid>_<secret>`, stored as a lowercase-hex SHA-256 digest in
//! `[local_api]`. This module is the verifier the API mount uses:
//!
//! - the raw header value is never logged;
//! - the decision is made against the CURRENT config snapshot, so a revoke lands
//!   on the next request (`update_config` publishes the new snapshot);
//! - a missing/unreadable config fails CLOSED;
//! - rejections are JSON, so API clients never receive the node manager's HTML
//!   login page or an HTML 404 from the static fallback.

use axum::Json;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use lqos_config::Config;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::local_api_keys::bytes_to_lower_hex;

/// The header the local API contract has always used.
const BEARER_HEADER: &str = "x-bearer";

/// Body returned for a rejected request.
#[derive(Debug, Serialize)]
struct ApiAuthError {
    error: &'static str,
}

fn unauthorised() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiAuthError {
            error: "missing or invalid x-bearer API key",
        }),
    )
        .into_response()
}

/// Compares two byte strings without early exit.
///
/// Length is not hidden (digests are a fixed 64 characters); content is.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (a, b) in left.iter().zip(right.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Whether `bearer` is a configured named key or the legacy token.
///
/// Pure: the caller supplies the config snapshot it considers authoritative.
pub(crate) fn authorises(config: &Config, bearer: &str) -> bool {
    let bearer = bearer.trim();
    if bearer.is_empty() {
        return false;
    }

    let digest = bytes_to_lower_hex(&Sha256::digest(bearer.as_bytes()));
    let mut authorised = false;
    for key in config.local_api.keys.iter() {
        authorised |= constant_time_eq(key.token_sha256.as_bytes(), digest.as_bytes());
    }
    if let Some(legacy) = config.local_api.bearer_token.as_deref() {
        authorised |= constant_time_eq(legacy.trim().as_bytes(), bearer.as_bytes());
    }
    authorised
}

/// Reads the request's `x-bearer` and decides against the current config.
fn request_authorised(req: &Request) -> bool {
    let Some(bearer) = req
        .headers()
        .get(BEARER_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    match lqos_config::load_config() {
        Ok(config) => authorises(config.as_ref(), bearer),
        // Fail closed: an unreadable config must not open the API.
        Err(_) => false,
    }
}

/// Middleware guarding the mounted API: key-only, no cookies, no licence keys.
///
/// A `User-Token` cookie session (the admin UI) deliberately does NOT authorise
/// these routes; the API surface is key-only by design.
pub(crate) async fn require_api_key(req: Request, next: Next) -> Response {
    if !request_authorised(&req) {
        return unauthorised();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::{authorises, constant_time_eq};
    use lqos_config::{Config, LocalApiKeyConfig};
    use sha2::{Digest, Sha256};

    fn config_with_key(key: &str) -> Config {
        let mut config = Config::default();
        config.local_api.keys = vec![LocalApiKeyConfig {
            id: "11111111-1111-4111-8111-111111111111".to_string(),
            name: "ext".to_string(),
            token_sha256: format!("{:x}", Sha256::digest(key.as_bytes())),
            created_at_unix: 1,
        }];
        config
    }

    #[test]
    fn accepts_a_configured_key() {
        let config = config_with_key("lqos_api_key_one");
        assert!(authorises(&config, "lqos_api_key_one"));
    }

    #[test]
    fn rejects_unknown_empty_and_near_miss_keys() {
        let config = config_with_key("lqos_api_key_one");
        assert!(!authorises(&config, "lqos_api_key_two"));
        assert!(!authorises(&config, ""));
        assert!(!authorises(&config, "   "));
        assert!(!authorises(&config, "lqos_api_key_on"));
        assert!(!authorises(&config, "lqos_api_key_one_"));
    }

    #[test]
    fn a_revoked_key_stops_authorising() {
        let mut config = config_with_key("lqos_api_key_one");
        assert!(authorises(&config, "lqos_api_key_one"));
        config.local_api.keys.clear();
        assert!(!authorises(&config, "lqos_api_key_one"));
    }

    #[test]
    fn accepts_the_legacy_bearer_token() {
        let mut config = Config::default();
        config.local_api.bearer_token = Some("legacy-token".to_string());
        assert!(authorises(&config, "legacy-token"));
        assert!(!authorises(&config, "legacy-token-2"));
    }

    #[test]
    fn an_empty_config_authorises_nothing() {
        let config = Config::default();
        assert!(!authorises(&config, ""));
        assert!(!authorises(&config, "anything"));
    }

    /// The mint→verify seam: the digest the key manager writes is the digest the
    /// verifier accepts, and one changed character is enough to reject.
    #[test]
    fn minted_keys_verify_and_a_one_character_mutation_does_not() {
        use super::super::local_api_keys::{KEY_RANDOM_BYTES, build_key};

        let mut config = Config::default();
        let (record, creation) = build_key(
            &config,
            "ext",
            uuid::Uuid::nil(),
            &[7_u8; KEY_RANDOM_BYTES],
            1,
        )
        .expect("mint a key");
        config.local_api.keys.push(record);

        assert!(
            authorises(&config, &creation.api_key),
            "the minted key must verify against the record the manager wrote"
        );

        let mut mutated = creation.api_key.clone();
        let last = mutated.pop().expect("a key has characters");
        mutated.push(if last == 'a' { 'b' } else { 'a' });
        assert!(
            !authorises(&config, &mutated),
            "a one-character mutation must not verify"
        );
    }

    #[test]
    fn cookie_sessions_are_not_keys() {
        // A `User-Token` cookie value is not a bearer key and must not match.
        let config = config_with_key("lqos_api_key_one");
        assert!(!authorises(&config, "User-Token=abc123"));
    }

    #[test]
    fn constant_time_eq_matches_normal_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }
}
