//! The adapter registry: exactly one directory of documents, validated as a
//! whole and activated atomically.
//!
//! There is no embedded fallback, no environment-variable override, and no
//! partial activation. The caller names the directory — normally the vault's
//! own directory plus `adapters` — and a load either replaces the active
//! registry with the complete validated set or leaves the previous one in
//! place and reports every failure.

use std::fmt;
use std::path::{Path, PathBuf};

use super::{Document, ValidationError, validate_registry};

/// The adapters directory a resolved vault file implies: the vault file's
/// directory plus `adapters`.
///
/// With no resolved vault path, the process working directory's `adapters`
/// is used, so a checkout can carry documents before a vault exists. This is
/// the only fallback: nothing is embedded in the binary and no environment
/// variable changes the location.
pub fn directory_for(vault_path: Option<&Path>) -> PathBuf {
    match vault_path {
        Some(path) => path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("adapters"),
        None => PathBuf::from("adapters"),
    }
}

/// What a registry load found: a validated document set, or the named
/// `unconfigured` state when no document is present.
#[derive(Debug, Default, Clone)]
pub enum RegistryState {
    /// The directory is missing, or holds no `*.json` document. Discovery
    /// reports `unconfigured` rather than a silent empty result.
    #[default]
    Unconfigured,
    /// The directory's documents, validated together.
    Loaded(Vec<Document>),
}

/// The active registry and the last failed load.
///
/// A failed [`reload`](Registry::reload) never changes the active state: the
/// last valid registry stays active and the failure is returned and kept for
/// the caller to report. A successful reload clears the recorded failure.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    state: RegistryState,
    last_failure: Option<RegistryError>,
}

impl Registry {
    /// Reads and validates `directory`, activating it only when every
    /// document is valid.
    ///
    /// A missing directory, or one with no `*.json` document, is
    /// [`RegistryState::Unconfigured`] and not an error. A directory that
    /// cannot be listed, an unreadable document, or any parse or validation
    /// failure returns the failures by document path and leaves the previous
    /// state active.
    pub fn reload(&mut self, directory: &Path) -> Result<(), RegistryError> {
        match read_registry(directory) {
            Ok(state) => {
                self.state = state;
                self.last_failure = None;
                Ok(())
            }
            Err(error) => {
                self.last_failure = Some(error.clone());
                Err(error)
            }
        }
    }

    /// The active state.
    pub fn state(&self) -> &RegistryState {
        &self.state
    }

    /// The active documents, in file-name order; empty when unconfigured.
    pub fn documents(&self) -> &[Document] {
        match &self.state {
            RegistryState::Unconfigured => &[],
            RegistryState::Loaded(documents) => documents,
        }
    }

    /// Whether a validated document set is active.
    pub fn is_configured(&self) -> bool {
        matches!(self.state, RegistryState::Loaded(_))
    }

    /// The last failed load, kept until a load succeeds.
    pub fn last_failure(&self) -> Option<&RegistryError> {
        self.last_failure.as_ref()
    }
}

/// Why a registry load failed. Nothing is activated on failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// The directory exists but cannot be listed.
    Directory {
        /// The directory the caller named.
        directory: PathBuf,
        /// The operating system's reason.
        reason: String,
    },
    /// One or more documents failed to parse or validate.
    Documents(Vec<ValidationError>),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Directory { directory, reason } => {
                write!(
                    f,
                    "cannot read adapter directory {}: {reason}",
                    directory.display()
                )
            }
            Self::Documents(errors) => {
                write!(f, "{} adapter document failure(s)", errors.len())?;
                for error in errors {
                    write!(f, "; {error}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// Reads and validates every `*.json` document directly in `directory`.
fn read_registry(directory: &Path) -> Result<RegistryState, RegistryError> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RegistryState::Unconfigured);
        }
        Err(error) => {
            return Err(RegistryError::Directory {
                directory: directory.to_owned(),
                reason: error.to_string(),
            });
        }
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut failures: Vec<ValidationError> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| RegistryError::Directory {
            directory: directory.to_owned(),
            reason: error.to_string(),
        })?;
        let path = entry.path();
        // Case-sensitive: only a name ending `.json` is a document. Anything
        // that carries that name and is not a readable regular file is a
        // named failure, never a quiet skip.
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        match path.metadata() {
            Ok(metadata) if metadata.is_file() => paths.push(path),
            Ok(_) => failures.push(unreadable_document(&path, "not a regular file")),
            Err(error) => failures.push(unreadable_document(
                &path,
                &format!("cannot read document: {error}"),
            )),
        }
    }
    if paths.is_empty() && failures.is_empty() {
        return Ok(RegistryState::Unconfigured);
    }
    paths.sort();

    let mut documents = Vec::new();
    for path in paths {
        let document = path.display().to_string();
        match std::fs::read_to_string(&path) {
            Ok(text) => match super::parse_document(&document, &text) {
                Ok(parsed) => documents.push((document, parsed)),
                Err(error) => failures.push(error),
            },
            Err(error) => failures.push(unreadable_document(
                &path,
                &format!("cannot read document: {error}"),
            )),
        }
    }
    if !failures.is_empty() {
        return Err(RegistryError::Documents(failures));
    }
    validate_registry(&documents).map_err(RegistryError::Documents)?;
    Ok(RegistryState::Loaded(
        documents
            .into_iter()
            .map(|(_, document)| document)
            .collect(),
    ))
}

/// The named failure for a `*.json` entry that cannot be read as a document.
fn unreadable_document(path: &Path, reason: &str) -> ValidationError {
    ValidationError {
        document: path.display().to_string(),
        adapter: None,
        probe: None,
        path: "file".to_owned(),
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
impl Registry {
    /// A registry over documents a test built directly. The loader and its
    /// error semantics have their own suite; this seam keeps a walk-source
    /// test from writing a scratch directory.
    pub(crate) fn from_documents(documents: Vec<Document>) -> Self {
        Self {
            state: RegistryState::Loaded(documents),
            last_failure: None,
        }
    }
}
