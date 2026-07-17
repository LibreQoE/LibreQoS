//! Administrative management for named local API credentials.

use crate::node_manager::auth::LoginResult;
use lqos_bus::{BusRequest, BusResponse, bus_request_with_timeout};
use lqos_config::{Config, LocalApiKeyConfig, MAX_LOCAL_API_KEYS};
use once_cell::sync::Lazy;
use rand::RngCore;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, MutexGuard};
use uuid::Uuid;

const CONFIG_BUS_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const KEY_NAME_MAX_CHARS: usize = 64;
const KEY_RANDOM_BYTES: usize = 32;
const KEY_ID_GENERATION_ATTEMPTS: usize = 8;
const KEY_PREFIX: &str = "lqos_api_";
static CONFIG_UPDATE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// One-time response returned after a named key is created.
///
/// This type intentionally does not implement `Debug` so the raw key cannot be
/// included accidentally in diagnostic output.
#[derive(Serialize)]
pub struct LocalApiKeyCreation {
    pub id: String,
    pub name: String,
    pub created_at_unix: u64,
    pub api_key: String,
}

fn normalize_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("API key name cannot be blank".to_string());
    }
    if trimmed.chars().count() > KEY_NAME_MAX_CHARS {
        return Err(format!(
            "API key name cannot exceed {KEY_NAME_MAX_CHARS} characters"
        ));
    }
    Ok(trimmed.to_string())
}

fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn valid_record(key: &LocalApiKeyConfig) -> bool {
    let valid_id =
        Uuid::parse_str(&key.id).is_ok_and(|parsed| parsed.hyphenated().to_string() == key.id);
    let valid_name = normalize_name(&key.name).is_ok_and(|normalized| normalized == key.name);
    let valid_digest = key.token_sha256.len() == 64
        && key
            .token_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    valid_id && valid_name && valid_digest && key.created_at_unix > 0
}

fn build_key(
    config: &Config,
    name: &str,
    id: Uuid,
    random_secret: &[u8; KEY_RANDOM_BYTES],
    created_at_unix: u64,
) -> Result<(LocalApiKeyConfig, LocalApiKeyCreation), String> {
    let name = normalize_name(name)?;
    if config.local_api.keys.len() >= MAX_LOCAL_API_KEYS {
        return Err(format!(
            "No more than {MAX_LOCAL_API_KEYS} named local API keys may be configured"
        ));
    }
    if config
        .local_api
        .keys
        .iter()
        .any(|key| key.name.to_lowercase() == name.to_lowercase())
    {
        return Err("An API key with that name already exists".to_string());
    }

    let id = id.hyphenated().to_string();
    if config.local_api.keys.iter().any(|key| key.id == id) {
        return Err("An API key with that ID already exists".to_string());
    }
    let api_key = format!("{KEY_PREFIX}{id}_{}", bytes_to_lower_hex(random_secret));
    let record = LocalApiKeyConfig {
        id: id.clone(),
        name: name.clone(),
        token_sha256: bytes_to_lower_hex(&Sha256::digest(api_key.as_bytes())),
        created_at_unix,
    };
    if !valid_record(&record) {
        return Err("Generated API key record failed validation".to_string());
    }
    Ok((
        record,
        LocalApiKeyCreation {
            id,
            name,
            created_at_unix,
            api_key,
        },
    ))
}

fn append_key(
    config: &mut Config,
    name: &str,
    id: Uuid,
    random_secret: &[u8; KEY_RANDOM_BYTES],
    created_at_unix: u64,
) -> Result<LocalApiKeyCreation, String> {
    let (record, creation) = build_key(config, name, id, random_secret, created_at_unix)?;
    config.local_api.keys.push(record);
    Ok(creation)
}

fn revoke_from_config(config: &mut Config, id: &str) -> Result<(), String> {
    let original_len = config.local_api.keys.len();
    config.local_api.keys.retain(|key| key.id != id);
    if config.local_api.keys.len() == original_len {
        return Err("API key was not found".to_string());
    }
    Ok(())
}

fn remove_legacy_from_config(config: &mut Config) -> Result<(), String> {
    config
        .local_api
        .bearer_token
        .take()
        .map(|_| ())
        .ok_or_else(|| "No legacy local API key is configured".to_string())
}

/// Serializes Node Manager configuration read-modify-write transactions.
pub(crate) async fn lock_config_update() -> MutexGuard<'static, ()> {
    CONFIG_UPDATE_LOCK.lock().await
}

/// Serializes a configuration transaction running on a dedicated blocking
/// thread, such as the remote-command worker.
pub(crate) fn lock_config_update_blocking() -> MutexGuard<'static, ()> {
    CONFIG_UPDATE_LOCK.blocking_lock()
}

/// Persists an updated configuration through `lqosd`'s local bus.
pub(super) async fn persist_config(config: Config) -> Result<(), String> {
    let mut responses = bus_request_with_timeout(
        vec![BusRequest::UpdateLqosdConfig(Box::new(config))],
        CONFIG_BUS_REQUEST_TIMEOUT,
    )
    .await
    .map_err(|error| format!("Unable to update config: {error}"))?;

    match responses.pop() {
        Some(BusResponse::Ack) => Ok(()),
        Some(BusResponse::Fail(message)) => Err(message),
        Some(other) => Err(format!("Unexpected config update response: {other:?}")),
        None => Err("No response received for config update".to_string()),
    }
}

/// Creates and persists a named local API key for an administrator.
///
/// Side effects: updates the active configuration. The raw key is returned only
/// by this successful call.
pub async fn create(login: LoginResult, name: String) -> Result<LocalApiKeyCreation, String> {
    if login != LoginResult::Admin {
        return Err("Unauthorized".to_string());
    }
    let _guard = lock_config_update().await;
    let mut config = lqos_config::load_config()
        .map_err(|_| "Unable to load the current config".to_string())?
        .as_ref()
        .clone();
    let id = (0..KEY_ID_GENERATION_ATTEMPTS)
        .map(|_| Uuid::new_v4())
        .find(|candidate| {
            let candidate = candidate.hyphenated().to_string();
            config.local_api.keys.iter().all(|key| key.id != candidate)
        })
        .ok_or_else(|| "Unable to allocate a unique API key ID".to_string())?;
    let mut random_secret = [0_u8; KEY_RANDOM_BYTES];
    rand::thread_rng().fill_bytes(&mut random_secret);
    let created_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock is before the Unix epoch".to_string())?
        .as_secs();
    let creation = append_key(&mut config, &name, id, &random_secret, created_at_unix)?;
    persist_config(config).await?;
    Ok(creation)
}

/// Revokes one named local API key for an administrator.
///
/// Side effects: updates the active configuration.
pub async fn revoke(login: LoginResult, id: String) -> Result<(), String> {
    if login != LoginResult::Admin {
        return Err("Unauthorized".to_string());
    }
    let canonical_id = Uuid::parse_str(id.trim())
        .map_err(|_| "Invalid API key ID".to_string())?
        .hyphenated()
        .to_string();
    if canonical_id != id.trim() {
        return Err("Invalid API key ID".to_string());
    }
    let _guard = lock_config_update().await;
    let mut config = lqos_config::load_config()
        .map_err(|_| "Unable to load the current config".to_string())?
        .as_ref()
        .clone();
    revoke_from_config(&mut config, &canonical_id)?;
    persist_config(config).await
}

/// Removes the legacy local API bearer token for an administrator.
///
/// Side effects: updates the active configuration.
pub async fn remove_legacy(login: LoginResult) -> Result<(), String> {
    if login != LoginResult::Admin {
        return Err("Unauthorized".to_string());
    }
    let _guard = lock_config_update().await;
    let mut config = lqos_config::load_config()
        .map_err(|_| "Unable to load the current config".to_string())?
        .as_ref()
        .clone();
    remove_legacy_from_config(&mut config)?;
    persist_config(config).await
}

#[cfg(test)]
mod tests {
    use super::{
        append_key, build_key, create, remove_legacy, remove_legacy_from_config, revoke,
        revoke_from_config,
    };
    use crate::node_manager::auth::LoginResult;
    use lqos_config::{Config, MAX_LOCAL_API_KEYS};
    use uuid::Uuid;

    #[test]
    fn generated_record_contains_only_digest_and_metadata() {
        let config = Config::default();
        let id = Uuid::from_u128(0x1234);
        let (record, creation) =
            build_key(&config, "  Monitor  ", id, &[0xab; 32], 42).expect("key should be valid");
        assert_eq!(record.name, "Monitor");
        assert_eq!(record.token_sha256.len(), 64);
        assert!(
            creation
                .api_key
                .starts_with(&format!("lqos_api_{}_", record.id))
        );
        assert!(creation.api_key.ends_with(&"ab".repeat(32)));
        let stored = serde_json::to_string(&record).expect("record should serialize");
        assert!(!stored.contains(&creation.api_key));
        assert_eq!(
            serde_json::to_value(&creation).expect("creation should serialize")["api_key"],
            creation.api_key
        );
    }

    #[test]
    fn validation_rejects_names_duplicates_ids_and_limit() {
        let mut config = Config::default();
        let secret = [7; 32];
        assert!(build_key(&config, " ", Uuid::from_u128(1), &secret, 1).is_err());
        assert!(build_key(&config, &"x".repeat(65), Uuid::from_u128(1), &secret, 1).is_err());
        let (first, _) = build_key(&config, "Monitor", Uuid::from_u128(1), &secret, 1).unwrap();
        config.local_api.keys.push(first);
        assert!(build_key(&config, " monitor ", Uuid::from_u128(2), &secret, 2).is_err());
        assert!(build_key(&config, "Other", Uuid::from_u128(1), &secret, 2).is_err());
        while config.local_api.keys.len() < MAX_LOCAL_API_KEYS {
            let n = config.local_api.keys.len() as u128 + 1;
            let (record, _) = build_key(
                &config,
                &format!("Key {n}"),
                Uuid::from_u128(n),
                &secret,
                n as u64,
            )
            .unwrap();
            config.local_api.keys.push(record);
        }
        assert!(build_key(&config, "Overflow", Uuid::from_u128(99), &secret, 99).is_err());
        assert!(
            build_key(
                &Config::default(),
                "Bad timestamp",
                Uuid::from_u128(100),
                &secret,
                0
            )
            .is_err()
        );
    }

    #[test]
    fn successful_mutations_preserve_other_credentials_and_store_no_raw_keys() {
        let mut config = Config::default();
        config.local_api.bearer_token = Some("legacy-token".to_string());
        let first = append_key(&mut config, "Monitor", Uuid::from_u128(1), &[1; 32], 1)
            .expect("first key should be created");
        let second = append_key(&mut config, "Automation", Uuid::from_u128(2), &[2; 32], 2)
            .expect("second key should be created");

        let stored = toml::to_string(&config).expect("config should serialize as TOML");
        assert!(!stored.contains(&first.api_key));
        assert!(!stored.contains(&second.api_key));
        assert!(stored.contains("token_sha256"));

        revoke_from_config(&mut config, &first.id).expect("first key should be revoked");
        assert_eq!(config.local_api.keys.len(), 1);
        assert_eq!(config.local_api.keys[0].id, second.id);
        assert_eq!(
            config.local_api.bearer_token.as_deref(),
            Some("legacy-token")
        );

        remove_legacy_from_config(&mut config).expect("legacy token should be removed");
        assert!(config.local_api.bearer_token.is_none());
        assert_eq!(config.local_api.keys.len(), 1);
    }

    #[tokio::test]
    async fn management_requires_an_administrator() {
        assert!(matches!(
            create(LoginResult::ReadOnly, "Monitor".into()).await,
            Err(message) if message == "Unauthorized"
        ));
        assert_eq!(
            revoke(LoginResult::Denied, Uuid::from_u128(1).to_string()).await,
            Err("Unauthorized".into())
        );
        assert_eq!(
            remove_legacy(LoginResult::ReadOnly).await,
            Err("Unauthorized".into())
        );
    }
}
