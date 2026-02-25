//! Autopilot error types.

use thiserror::Error;

/// Errors returned by Autopilot APIs.
#[derive(Error, Debug)]
pub enum AutopilotError {
    /// The Autopilot actor thread could not be spawned.
    #[error("failed to spawn Autopilot actor thread: {0}")]
    SpawnThread(#[from] std::io::Error),

    /// The overrides file could not be loaded.
    #[error("failed to load overrides file: {details}")]
    OverridesLoad { details: String },

    /// The overrides file could not be saved.
    #[error("failed to save overrides file: {details}")]
    OverridesSave { details: String },
}
