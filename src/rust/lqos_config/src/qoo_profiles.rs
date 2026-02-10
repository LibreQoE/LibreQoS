//! Loads and validates Quality-of-Outcome (QoO) profiles from `qoo_profiles.json`.
//!
//! Profiles are expected to live in `(<lqos_directory>)/qoo_profiles.json`.

use arc_swap::ArcSwap;
use lqos_utils::qoo::{ProfileIoError, QooProfile, QooProfilesFile};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use thiserror::Error;

/// Default QoO profile id when config does not specify one.
pub const DEFAULT_QOO_PROFILE_ID: &str = "web_browsing";

#[derive(Clone)]
struct CachedProfiles {
    path: PathBuf,
    modified: Option<SystemTime>,
    file: Arc<QooProfilesFile>,
}

static PROFILES: Lazy<ArcSwap<Option<Arc<CachedProfiles>>>> = Lazy::new(|| ArcSwap::from_pointee(None));

/// Minimal profile metadata for UI selection lists.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QooProfileInfo {
    /// Profile id (stable key, stored in config).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Errors encountered while loading or selecting QoO profiles.
#[derive(Debug, Error)]
pub enum QooProfilesError {
    /// Unable to load LibreQoS config (needed to locate `lqos_directory`).
    #[error("Unable to load LibreQoS configuration")]
    ConfigLoad(#[from] crate::etc::LibreQoSConfigError),

    /// `qoo_profiles.json` not found.
    #[error("QoO profiles file not found at {0}")]
    FileNotFound(String),

    /// Profile file I/O, JSON parse, or validation error.
    #[error("{0}")]
    ProfileIo(#[from] ProfileIoError),

    /// No profiles exist in the file.
    #[error("QoO profiles file contains no profiles")]
    EmptyProfiles,
}

fn profiles_path_from_config() -> Result<PathBuf, QooProfilesError> {
    let cfg = crate::load_config()?;
    Ok(Path::new(&cfg.lqos_directory).join("qoo_profiles.json"))
}

fn metadata_modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

fn load_profiles_from_disk(path: &Path) -> Result<Arc<QooProfilesFile>, QooProfilesError> {
    if !path.exists() {
        return Err(QooProfilesError::FileNotFound(path.display().to_string()));
    }
    let file = QooProfilesFile::load_json(path)?;
    if file.profiles.is_empty() {
        return Err(QooProfilesError::EmptyProfiles);
    }
    Ok(Arc::new(file))
}

/// Load and cache the QoO profile table from disk.
///
/// The cache is automatically invalidated when the file modification time changes or
/// when `lqos_directory` changes.
pub fn load_qoo_profiles_file() -> Result<Arc<QooProfilesFile>, QooProfilesError> {
    let path = profiles_path_from_config()?;
    let modified = metadata_modified(&path);

    if let Some(cached) = PROFILES.load().as_ref() {
        if cached.path == path && cached.modified == modified {
            return Ok(cached.file.clone());
        }
    }

    let file = load_profiles_from_disk(&path)?;
    PROFILES.store(
        Some(Arc::new(CachedProfiles {
            path,
            modified,
            file: file.clone(),
        }))
        .into(),
    );

    Ok(file)
}

/// List available QoO profiles (id/name/description) for UI selection.
pub fn list_qoo_profiles() -> Result<Vec<QooProfileInfo>, QooProfilesError> {
    let file = load_qoo_profiles_file()?;
    Ok(file
        .profiles
        .iter()
        .map(|p| QooProfileInfo {
            id: p.id.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
        })
        .collect())
}

/// Select the active QoO profile based on config (fallbacks to `DEFAULT_QOO_PROFILE_ID`).
pub fn active_qoo_profile() -> Result<Arc<QooProfile>, QooProfilesError> {
    let cfg = crate::load_config()?;
    let requested = cfg
        .qoo_profile_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let file = load_qoo_profiles_file()?;

    let pick = |id: &str| file.profiles.iter().find(|p| p.id == id);

    let selected = requested
        .and_then(pick)
        .or_else(|| pick(DEFAULT_QOO_PROFILE_ID))
        .or_else(|| file.pick_default())
        .ok_or(QooProfilesError::EmptyProfiles)?;

    Ok(Arc::new(selected.to_runtime()))
}

