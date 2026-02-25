//! Autopilot error types.

use thiserror::Error;

/// Errors returned by Autopilot APIs.
#[derive(Error, Debug)]
pub enum AutopilotError {
    /// The Autopilot actor thread could not be spawned.
    #[error("failed to spawn Autopilot actor thread: {0}")]
    SpawnThread(#[from] std::io::Error),
}

