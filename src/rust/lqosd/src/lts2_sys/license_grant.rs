use anyhow::{Context, Result, bail};
use dryoc::sign::{PublicKey, SecretKey, Signature, SignedMessage, SigningKeyPair};
use dryoc::types::Bytes;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::lts2_sys::lts2_client::{LicenseStatus, set_license_status};
use lqos_utils::unix_time::unix_now;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const KEY_DIR: &str = ".keys";
const KEYPAIR_FILE: &str = "keypair";
const LICENSE_GRANT_FILE: &str = "insight_license";

#[derive(Clone, Serialize, Deserialize)]
pub struct StoredKeypair {
    pub local_keypair: SigningKeyPair<PublicKey, SecretKey>,
    pub insight_public_key: Option<PublicKey>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct LicenseGrant {
    pub license_state: i32,
    pub trial_expiration: i64,
    pub grant_expires: i64,
    pub issued_at: i64,
    pub license_uuid: Option<Uuid>,
    pub node_id: Option<String>,
    pub max_circuits: Option<u64>,
    pub lqosd_public_key: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LicenseGrantEnvelope {
    pub payload: Vec<u8>,
    pub signature: Vec<u8>,
}

static KEYPAIR_STATE: OnceCell<Mutex<StoredKeypair>> = OnceCell::new();
static KEYPAIR_PATH: OnceCell<PathBuf> = OnceCell::new();
static GRANT_PATH: OnceCell<PathBuf> = OnceCell::new();
static GRANT_STATE: OnceCell<Mutex<Option<LicenseGrant>>> = OnceCell::new();

pub fn init_license_storage(config: &lqos_config::Config) -> Result<()> {
    let key_dir = Path::new(&config.lqos_directory).join(KEY_DIR);
    let key_path = key_dir.join(KEYPAIR_FILE);
    let grant_path = key_dir.join(LICENSE_GRANT_FILE);

    KEYPAIR_PATH.get_or_init(|| key_path.clone());
    GRANT_PATH.get_or_init(|| grant_path.clone());
    GRANT_STATE.get_or_init(|| Mutex::new(None));

    let keypair = load_or_create_keypair(&key_path)?;
    KEYPAIR_STATE.get_or_init(|| Mutex::new(keypair));

    load_offline_grant()?;
    Ok(())
}

pub fn local_public_key_bytes() -> Option<Vec<u8>> {
    KEYPAIR_STATE
        .get()
        .map(|state| state.lock().local_keypair.public_key.as_slice().to_vec())
}

pub fn update_insight_public_key_bytes(public_key: Vec<u8>) -> Result<()> {
    let key = PublicKey::try_from(public_key.as_slice())
        .context("Insight public key size mismatch")?;
    update_insight_public_key(key)
}

pub fn handle_license_grant(payload: Vec<u8>, signature: Vec<u8>) -> Result<()> {
    let envelope = LicenseGrantEnvelope { payload, signature };
    let now = unix_now().unwrap_or(0) as i64;
    let grant = verify_grant_with_time(&envelope, now)?;
    save_license_grant(&envelope)?;
    store_grant_state(Some(grant.clone()));
    apply_license_status(&grant);
    info!(
        "Insight license grant stored (expires={}, license_state={})",
        grant.grant_expires, grant.license_state
    );
    Ok(())
}

pub fn invalidate_license_grant() -> Result<()> {
    let grant_path = GRANT_PATH
        .get()
        .context("License grant path not initialized")?;
    if grant_path.exists() {
        fs::remove_file(grant_path).ok();
        info!("Deleted stale Insight license grant");
    }
    store_grant_state(None);
    set_license_status(LicenseStatus {
        license_type: 0,
        trial_expires: -1,
    });
    Ok(())
}

pub fn purge_license_grant_file() -> Result<()> {
    let grant_path = GRANT_PATH
        .get()
        .context("License grant path not initialized")?;
    if grant_path.exists() {
        fs::remove_file(grant_path).ok();
        info!("Deleted Insight license grant");
    }
    store_grant_state(None);
    Ok(())
}

fn apply_license_status(grant: &LicenseGrant) {
    set_license_status(LicenseStatus {
        license_type: grant.license_state,
        trial_expires: grant.trial_expiration as i32,
    });
}

fn load_or_create_keypair(path: &Path) -> Result<StoredKeypair> {
    if path.exists() {
        let bytes = fs::read(path).context("Failed to read Insight keypair")?;
        let keypair = serde_cbor::from_slice(&bytes).context("Failed to decode Insight keypair")?;
        debug!("Loaded Insight keypair from {}", path.display());
        return Ok(keypair);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create key directory")?;
    }

    let keypair = StoredKeypair {
        local_keypair: SigningKeyPair::gen_with_defaults(),
        insight_public_key: None,
    };
    save_keypair(path, &keypair)?;
    debug!("Generated Insight keypair at {}", path.display());
    Ok(keypair)
}

fn save_keypair(path: &Path, keypair: &StoredKeypair) -> Result<()> {
    let bytes = serde_cbor::to_vec(keypair).context("Failed to encode Insight keypair")?;
    write_atomic(path, &bytes)
}

fn update_insight_public_key(key: PublicKey) -> Result<()> {
    let state = KEYPAIR_STATE
        .get()
        .context("Keypair state not initialized")?;
    let mut stored = state.lock();
    let matches = stored
        .insight_public_key
        .as_ref()
        .map(|existing| existing.as_slice() == key.as_slice())
        .unwrap_or(false);
    if matches {
        return Ok(());
    }
    stored.insight_public_key = Some(key);
    let path = KEYPAIR_PATH
        .get()
        .context("Keypair path not initialized")?;
    save_keypair(path, &stored)?;
    info!("Stored Insight public key");
    Ok(())
}

fn load_offline_grant() -> Result<()> {
    let grant_path = GRANT_PATH
        .get()
        .context("License grant path not initialized")?;
    if !grant_path.exists() {
        return Ok(());
    }
    let bytes = fs::read(grant_path).context("Failed to read license grant")?;
    let envelope: LicenseGrantEnvelope =
        serde_cbor::from_slice(&bytes).context("Failed to decode license grant")?;
    let now = unix_now().unwrap_or(0) as i64;
    match verify_grant_with_time(&envelope, now) {
        Ok(grant) => {
            info!(
                "Loaded offline license grant (expires={}, license_state={})",
                grant.grant_expires, grant.license_state
            );
            store_grant_state(Some(grant.clone()));
            apply_license_status(&grant);
        }
        Err(err) => {
            warn!("Offline license grant invalid: {}", err);
            invalidate_license_grant().ok();
        }
    }
    Ok(())
}

pub fn current_license_summary() -> (Option<Uuid>, Option<u64>) {
    let Some(state) = GRANT_STATE.get() else {
        return (None, None);
    };
    let guard = state.lock();
    let Some(grant) = guard.as_ref() else {
        return (None, None);
    };
    (grant.license_uuid, grant.max_circuits)
}

pub fn current_license_limits() -> (bool, Option<u64>) {
    let Some(state) = GRANT_STATE.get() else {
        return (false, None);
    };
    let guard = state.lock();
    let Some(grant) = guard.as_ref() else {
        return (false, None);
    };
    let now = unix_now().unwrap_or(0) as i64;
    if grant.grant_expires <= now {
        return (false, None);
    }
    (true, grant.max_circuits)
}

fn store_grant_state(grant: Option<LicenseGrant>) {
    let state = GRANT_STATE.get_or_init(|| Mutex::new(None));
    *state.lock() = grant;
}

fn save_license_grant(envelope: &LicenseGrantEnvelope) -> Result<()> {
    let grant_path = GRANT_PATH
        .get()
        .context("License grant path not initialized")?;
    let bytes = serde_cbor::to_vec(envelope).context("Failed to encode license grant")?;
    write_atomic(grant_path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let Some(parent) = path.parent() else {
        bail!("Invalid path for atomic write");
    };
    fs::create_dir_all(parent).context("Failed to create parent directory")?;
    let tmp_path = path.with_extension("tmp");

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp_path)
        .context("Failed to open temp file for write")?;
    file.write_all(bytes).context("Failed to write temp file")?;
    file.sync_all().ok();
    #[cfg(unix)]
    fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600)).ok();
    fs::rename(&tmp_path, path).context("Failed to rename temp file")?;
    Ok(())
}

fn verify_grant_with_time(envelope: &LicenseGrantEnvelope, now: i64) -> Result<LicenseGrant> {
    let state = KEYPAIR_STATE
        .get()
        .context("Keypair state not initialized")?;
    let stored = state.lock();
    let insight_key = stored
        .insight_public_key
        .as_ref()
        .context("Insight public key not available")?
        .clone();
    let local_pub = stored.local_keypair.public_key.as_slice().to_vec();
    drop(stored);

    verify_grant_with_time_and_keys(envelope, now, &insight_key, &local_pub)
}

fn verify_grant_with_time_and_keys(
    envelope: &LicenseGrantEnvelope,
    now: i64,
    insight_key: &PublicKey,
    local_pub: &[u8],
) -> Result<LicenseGrant> {
    let signature = Signature::try_from(envelope.signature.as_slice())
        .context("License grant signature size mismatch")?;
    let signed = SignedMessage::from_parts(signature, envelope.payload.clone());
    signed
        .verify(insight_key)
        .context("License grant signature invalid")?;
    let grant: LicenseGrant =
        serde_cbor::from_slice(&envelope.payload).context("License grant payload invalid")?;

    if grant.grant_expires <= now {
        bail!("License grant expired");
    }
    if grant.lqosd_public_key != local_pub {
        bail!("License grant bound to a different keypair");
    }
    Ok(grant)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_grant_roundtrip() {
        let signer = SigningKeyPair::gen_with_defaults();
        let lqosd = SigningKeyPair::gen_with_defaults();
        let now = 1_700_000_000i64;
        let grant = LicenseGrant {
            license_state: 3,
            trial_expiration: 0,
            grant_expires: now + 3600,
            issued_at: now,
            license_uuid: Some(Uuid::new_v4()),
            node_id: Some("node-1".to_string()),
            max_circuits: Some(1500),
            lqosd_public_key: lqosd.public_key.as_slice().to_vec(),
        };
        let envelope = {
            let payload = serde_cbor::to_vec(&grant).unwrap();
            let signed = signer.sign_with_defaults(payload.clone()).unwrap();
            let (signature, _message) = signed.into_parts();
            LicenseGrantEnvelope {
                payload,
                signature: signature.as_slice().to_vec(),
            }
        };
        let verified = verify_grant_with_time_and_keys(
            &envelope,
            now,
            &signer.public_key,
            lqosd.public_key.as_slice(),
        )
        .unwrap();
        assert_eq!(verified.license_state, grant.license_state);
    }

    #[test]
    fn license_grant_tamper_rejected() {
        let signer = SigningKeyPair::gen_with_defaults();
        let lqosd = SigningKeyPair::gen_with_defaults();
        let now = 1_700_000_000i64;
        let grant = LicenseGrant {
            license_state: 3,
            trial_expiration: 0,
            grant_expires: now + 3600,
            issued_at: now,
            license_uuid: None,
            node_id: None,
            max_circuits: None,
            lqosd_public_key: lqosd.public_key.as_slice().to_vec(),
        };
        let payload = serde_cbor::to_vec(&grant).unwrap();
        let signed = signer.sign_with_defaults(payload.clone()).unwrap();
        let (signature, _message) = signed.into_parts();
        let mut tampered = payload.clone();
        tampered[0] ^= 0x01;
        let envelope = LicenseGrantEnvelope {
            payload: tampered,
            signature: signature.as_slice().to_vec(),
        };
        assert!(verify_grant_with_time_and_keys(
            &envelope,
            now,
            &signer.public_key,
            lqosd.public_key.as_slice(),
        )
        .is_err());
    }

    #[test]
    fn license_grant_wrong_key_rejected() {
        let signer = SigningKeyPair::gen_with_defaults();
        let wrong_signer = SigningKeyPair::gen_with_defaults();
        let lqosd = SigningKeyPair::gen_with_defaults();
        let now = 1_700_000_000i64;
        let grant = LicenseGrant {
            license_state: 3,
            trial_expiration: 0,
            grant_expires: now + 3600,
            issued_at: now,
            license_uuid: None,
            node_id: None,
            max_circuits: None,
            lqosd_public_key: lqosd.public_key.as_slice().to_vec(),
        };
        let payload = serde_cbor::to_vec(&grant).unwrap();
        let signed = signer.sign_with_defaults(payload.clone()).unwrap();
        let (signature, _message) = signed.into_parts();
        let envelope = LicenseGrantEnvelope {
            payload,
            signature: signature.as_slice().to_vec(),
        };
        assert!(verify_grant_with_time_and_keys(
            &envelope,
            now,
            &wrong_signer.public_key,
            lqosd.public_key.as_slice(),
        )
        .is_err());
    }
}
