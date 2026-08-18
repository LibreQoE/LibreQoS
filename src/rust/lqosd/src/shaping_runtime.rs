use lqos_utils::unix_time::unix_now;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShapingRuntimeState {
    Active,
    Starting,
    Inactive,
    ErrorPreflight,
    ErrorKernelAttach,
    ErrorInterfaceMissing,
    ErrorConfig,
    Degraded,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShapingRuntimeStatus {
    pub state: ShapingRuntimeState,
    pub degraded: bool,
    pub can_retry: bool,
    pub summary: String,
    pub detail: Option<String>,
    pub updated_unix: Option<u64>,
}

impl Default for ShapingRuntimeStatus {
    fn default() -> Self {
        Self {
            state: ShapingRuntimeState::Starting,
            degraded: false,
            can_retry: false,
            summary: "LibreQoS is starting shaping.".to_string(),
            detail: None,
            updated_unix: None,
        }
    }
}

static SHAPING_RUNTIME_STATUS: Lazy<RwLock<ShapingRuntimeStatus>> =
    Lazy::new(|| RwLock::new(ShapingRuntimeStatus::default()));

fn now_unix() -> Option<u64> {
    unix_now().ok()
}

fn set_status(status: ShapingRuntimeStatus) {
    *SHAPING_RUNTIME_STATUS.write() = status;
}

pub fn mark_starting(summary: impl Into<String>) {
    set_status(ShapingRuntimeStatus {
        state: ShapingRuntimeState::Starting,
        degraded: false,
        can_retry: false,
        summary: summary.into(),
        detail: None,
        updated_unix: now_unix(),
    });
}

pub fn mark_active(summary: impl Into<String>) {
    set_status(ShapingRuntimeStatus {
        state: ShapingRuntimeState::Active,
        degraded: false,
        can_retry: false,
        summary: summary.into(),
        detail: None,
        updated_unix: now_unix(),
    });
}

pub fn mark_error(
    state: ShapingRuntimeState,
    summary: impl Into<String>,
    detail: impl Into<String>,
) {
    set_status(ShapingRuntimeStatus {
        state,
        degraded: true,
        can_retry: true,
        summary: summary.into(),
        detail: Some(detail.into()),
        updated_unix: now_unix(),
    });
}

pub fn get_status() -> ShapingRuntimeStatus {
    SHAPING_RUNTIME_STATUS.read().clone()
}

pub fn classify_preflight_error(detail: &str) -> ShapingRuntimeState {
    let lower = detail.to_lowercase();
    if lower.contains("does not exist") || lower.contains("missing") {
        ShapingRuntimeState::ErrorInterfaceMissing
    } else if lower.contains("configuration")
        || lower.contains("/etc/lqos.conf")
        || lower.contains("load config")
    {
        ShapingRuntimeState::ErrorConfig
    } else {
        ShapingRuntimeState::ErrorPreflight
    }
}
