use lqos_config::Config;
use serde::{Deserialize, Serialize};

/// Input for a staged helper apply transaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApplyRequest {
    pub config: Config,
    pub source: String,
    pub operator_username: Option<String>,
    #[serde(default)]
    pub mode: ApplyMode,
    #[serde(default)]
    pub confirm_dangerous_changes: bool,
}

/// Apply-mode variants used by the transaction engine.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApplyMode {
    #[default]
    Apply,
    Adopt,
    TakeOver,
}

/// Generic helper action response used by apply/confirm/revert/rollback/retry-shaping.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ApplyResponse {
    pub ok: bool,
    pub message: String,
    pub operation: Option<PendingOperationStatus>,
    pub last_backup_id: Option<String>,
}

/// High-level helper status returned to callers.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HelperStatus {
    #[serde(default)]
    pub pending_operation: Option<PendingOperationStatus>,
    #[serde(default)]
    pub last_backup_id: Option<String>,
    #[serde(default)]
    pub recent_backup_ids: Vec<String>,
    #[serde(default)]
    pub recent_backups: Vec<BackupSummary>,
}

/// Serializable description of a pending network change awaiting confirmation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PendingOperationStatus {
    pub operation_id: String,
    pub backup_id: String,
    pub state: String,
    pub source: String,
    pub operator_username: Option<String>,
    pub summary: String,
    pub created_unix: u64,
}

/// Summary of a stored rollback bundle.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BackupSummary {
    pub backup_id: String,
    pub timestamp_unix: u64,
    pub source: String,
    pub operator_username: Option<String>,
    pub old_mode: String,
    pub new_mode: String,
    #[serde(default)]
    pub warnings_present: Vec<String>,
}
