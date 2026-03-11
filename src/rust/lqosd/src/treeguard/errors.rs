//! TreeGuard error types.

use thiserror::Error;

/// Errors returned by TreeGuard APIs.
#[derive(Error, Debug)]
pub enum TreeguardError {
    /// The TreeGuard actor thread could not be spawned.
    #[error("failed to spawn TreeGuard actor thread: {0}")]
    SpawnThread(#[from] std::io::Error),

    /// The overrides file could not be loaded.
    #[error("failed to load overrides file: {details}")]
    OverridesLoad { details: String },

    /// The overrides file could not be saved.
    #[error("failed to save overrides file: {details}")]
    OverridesSave { details: String },

    /// Bakery is not initialized.
    #[error("bakery is not initialized")]
    BakeryNotReady,

    /// Failed to send a command to Bakery.
    #[error("failed to send command to bakery: {details}")]
    BakerySend { details: String },

    /// The queue structure snapshot was unavailable.
    #[error("queue structure snapshot unavailable: {details}")]
    QueueStructureUnavailable { details: String },

    /// The circuit was not found in the queue structure snapshot.
    #[error("circuit not found in queue structure: {circuit_id}")]
    CircuitNotFound { circuit_id: String },

    /// A queue structure class identifier was invalid.
    #[error("invalid queue structure class id: {details}")]
    InvalidClassId { details: String },
}
