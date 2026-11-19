//! Placeholder vault: loads targets, credentials, credential profiles,
//! and the scan policy from a read-only JSON file resolved at runtime.
//!
//! The conventions are the top-level `targets` and `credentials` arrays
//! (permissive, free-form entries with a `name` field for lookup), the
//! top-level `profiles` array, whose entries [`crate::profiles`] parses
//! into typed [`Profile`]s at load time, and the optional top-level
//! `scan_policy` object, whose allow/ignore rules parse into a typed
//! [`ScanPolicy`] at load time. Everything else — entry fields and
//! unknown top-level keys — is preserved verbatim so the file shape
//! can churn with zero code changes. This is not a secret store: no
//! grants, no references, no mutation.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::net_policy::{ScanPolicy, ScanPolicyError};
use crate::profiles::{Profile, ProfileError, parse_all};

/// The vault file name resolved in the working directory and beside the
/// executable.
const VAULT_FILE_NAME: &str = "vault.json";

/// Resolves the vault file at runtime — no build-machine path is ever
/// baked into the binary: the `DUDARCHY_VAULT` environment variable wins
/// when set to a non-empty value (used as-is, so a typo surfaces as a
/// load error for the exact path), else `vault.json` in the current
/// working directory, else `vault.json` beside the executable. The error
/// names every location searched, so a missing vault is never a mystery.
pub fn default_vault_path() -> Result<PathBuf, VaultError> {
    if let Some(path) = std::env::var_os("DUDARCHY_VAULT").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let mut searched: Vec<String> = Vec::new();
    if let Ok(dir) = std::env::current_dir() {
        let candidate = dir.join(VAULT_FILE_NAME);
        if candidate.is_file() {
            return Ok(candidate);
        }
        searched.push(candidate.display().to_string());
    } else {
        searched.push("the current working directory (unavailable)".to_owned());
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join(VAULT_FILE_NAME);
        if candidate.is_file() {
            return Ok(candidate);
        }
        searched.push(candidate.display().to_string());
    }
    Err(VaultError::NotFound {
        searched: searched.join(", "),
    })
}

/// A loaded vault: free-form JSON entries under the `targets` and
/// `credentials` conventions, typed [`Profile`]s under `profiles`, the
/// optional typed [`ScanPolicy`] under `scan_policy`, plus any other
/// top-level keys verbatim.
#[derive(Debug)]
pub struct Vault {
    targets: Vec<Value>,
    credentials: Vec<Value>,
    profiles: Vec<Profile>,
    scan_policy: Option<ScanPolicy>,
    extra: Map<String, Value>,
}

impl Vault {
    /// Reads the vault JSON file at `path` and parses it.
    pub fn load(path: &Path) -> Result<Vault, VaultError> {
        let text = std::fs::read_to_string(path).map_err(|source| VaultError::Io {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(&text)
    }

    /// Parses vault JSON text. The top level must be an object; `targets`,
    /// `credentials`, and `profiles`, when present, must be arrays and may
    /// otherwise be absent (`profiles` entries must additionally parse as
    /// well-formed profiles); `scan_policy`, when present, must be a
    /// well-formed policy object (a non-empty `allow` array, an optional
    /// `ignore` array, no unknown fields). Every other top-level key is
    /// preserved verbatim.
    pub fn parse(json: &str) -> Result<Vault, VaultError> {
        let Value::Object(mut root) =
            serde_json::from_str::<Value>(json).map_err(VaultError::Parse)?
        else {
            return Err(VaultError::TopLevel);
        };
        let targets = take_array(&mut root, "targets")?;
        let credentials = take_array(&mut root, "credentials")?;
        let raw_profiles = take_array(&mut root, "profiles")?;
        let profiles = parse_all(&raw_profiles)?;
        let scan_policy = take_scan_policy(&mut root)?;
        Ok(Self {
            targets,
            credentials,
            profiles,
            scan_policy,
            extra: root,
        })
    }

    /// All entries under the top-level `targets` array, in file order.
    pub fn targets(&self) -> &[Value] {
        &self.targets
    }

    /// All entries under the top-level `credentials` array, in file order.
    pub fn credentials(&self) -> &[Value] {
        &self.credentials
    }

    /// All parsed credential profiles under the top-level `profiles` array,
    /// in file order.
    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    /// The first target entry whose `name` field is the string `name`.
    pub fn target(&self, name: &str) -> Option<&Value> {
        find_named(&self.targets, name)
    }

    /// The first credential entry whose `name` field is the string `name`.
    pub fn credential(&self, name: &str) -> Option<&Value> {
        find_named(&self.credentials, name)
    }

    /// The parsed scan policy under the top-level `scan_policy` object:
    /// `None` when the key is absent, which means scanning is not
    /// address-restricted.
    pub fn scan_policy(&self) -> Option<&ScanPolicy> {
        self.scan_policy.as_ref()
    }

    /// Top-level keys other than `targets`, `credentials`, `profiles`,
    /// and `scan_policy`, preserved verbatim.
    pub fn extra(&self) -> &Map<String, Value> {
        &self.extra
    }
}

/// Removes the optional top-level `scan_policy` object and parses it
/// into a typed [`ScanPolicy`]: `allow` is required and must be a
/// non-empty array of rule strings, `ignore` is optional and defaults to
/// empty, and no other field is accepted — a typo such as
/// `ignore_list` must fail loudly instead of silently disabling the
/// guard.
fn take_scan_policy(root: &mut Map<String, Value>) -> Result<Option<ScanPolicy>, VaultError> {
    const KEY: &str = "scan_policy";
    const ALLOW: &str = "scan_policy.allow";
    const IGNORE: &str = "scan_policy.ignore";
    let fail = |message: String| VaultError::ScanPolicy {
        message,
        source: None,
    };
    let Some(value) = root.remove(KEY) else {
        return Ok(None);
    };
    let Value::Object(object) = value else {
        return Err(fail("must be a JSON object".to_owned()));
    };
    for key in object.keys() {
        if key != "allow" && key != "ignore" {
            return Err(fail(format!("unknown field `{key}`")));
        }
    }
    let allow = match object.get("allow") {
        Some(entries) => string_entries(entries, ALLOW)?,
        None => return Err(fail(format!("{ALLOW} is required"))),
    };
    if allow.is_empty() {
        return Err(fail(format!(
            "{ALLOW} must not be empty — an empty allow list would deny every address, which is indistinguishable from a truncated list"
        )));
    }
    let ignore = match object.get("ignore") {
        Some(entries) => string_entries(entries, IGNORE)?,
        None => Vec::new(),
    };
    ScanPolicy::new(&allow, &ignore).map(Some).map_err(|error| {
        let message = error.to_string();
        VaultError::ScanPolicy {
            message,
            source: Some(error),
        }
    })
}

/// Reads a JSON array of rule strings.
fn string_entries(value: &Value, key: &str) -> Result<Vec<String>, VaultError> {
    let Value::Array(entries) = value else {
        return Err(VaultError::ScanPolicy {
            message: format!("`{key}` must be an array"),
            source: None,
        });
    };
    entries
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| VaultError::ScanPolicy {
                    message: format!("`{key}` entries must be strings"),
                    source: None,
                })
        })
        .collect()
}

/// Removes `key` from the vault root as an array; an absent key yields an
/// empty list.
fn take_array(root: &mut Map<String, Value>, key: &'static str) -> Result<Vec<Value>, VaultError> {
    match root.remove(key) {
        Some(Value::Array(entries)) => Ok(entries),
        Some(_) => Err(VaultError::NotAnArray { key }),
        None => Ok(Vec::new()),
    }
}

/// Finds the first entry whose `name` field is the string `name`.
fn find_named<'a>(entries: &'a [Value], name: &str) -> Option<&'a Value> {
    entries
        .iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
}

/// Failures that can occur while loading or parsing a vault file.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// The vault file could not be read.
    #[error("failed to read vault file from {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The vault file is not valid JSON.
    #[error("vault is not valid JSON")]
    Parse(#[source] serde_json::Error),
    /// The vault's top level was not a JSON object.
    #[error("vault top level must be a JSON object")]
    TopLevel,
    /// A conventional top-level key was present but not an array.
    #[error("vault key `{key}` must be an array")]
    NotAnArray { key: &'static str },
    /// A `profiles` entry is malformed; the source names the profile.
    #[error(transparent)]
    Profile(#[from] ProfileError),
    /// The top-level `scan_policy` object is malformed, or its rules
    /// failed to parse; the message names the offending key, entry, or
    /// rule, and the source carries the typed rule error when there is
    /// one.
    #[error("vault scan_policy: {message}")]
    ScanPolicy {
        /// The full reason, naming the offending key, entry, or rule.
        message: String,
        /// The underlying rule-parse error, when the object shape was
        /// valid.
        #[source]
        source: Option<ScanPolicyError>,
    },
    /// No vault file exists in any of the searched locations; the message
    /// names them.
    #[error(
        "no vault file found (searched {searched}); set DUDARCHY_VAULT, or place vault.json in the working directory or beside the executable"
    )]
    NotFound {
        /// The locations the resolver looked at, comma-separated.
        searched: String,
    },
}
