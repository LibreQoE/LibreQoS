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

fn active_status(summary: String) -> ShapingRuntimeStatus {
    ShapingRuntimeStatus {
        state: ShapingRuntimeState::Active,
        degraded: false,
        can_retry: false,
        summary,
        detail: None,
        updated_unix: now_unix(),
    }
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

/// Announces that shaping is active.
///
/// Ordering invariant (`.35` failure-surfacing): `set_status` is
/// last-writer-wins, and the end-of-startup announcement runs *after* any
/// recoverable failure was recorded. A recorded failure (`mark_error` sets
/// `degraded`) therefore outranks a generic "active" announcement — otherwise
/// this later write would erase the operator-visible error. A failure is
/// cleared only by the explicit [`mark_recovered`] success (or a fresh
/// [`mark_starting`] attempt).
pub fn mark_active(summary: impl Into<String>) {
    let summary = summary.into();
    let mut status = SHAPING_RUNTIME_STATUS.write();
    if status.degraded {
        return;
    }
    *status = active_status(summary);
}

/// Records an explicit success: clears any latched failure and reports Active.
pub fn mark_recovered(summary: impl Into<String>) {
    set_status(active_status(summary.into()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::failure_surfacing_test_lock;

    fn reset() {
        set_status(ShapingRuntimeStatus::default());
    }

    #[test]
    fn failure_survives_mark_active_and_recovery_clears_it() {
        let _guard = failure_surfacing_test_lock()
            .lock()
            .expect("failure-surfacing test lock should not be poisoned");
        reset();

        mark_error(
            ShapingRuntimeState::Degraded,
            "StormGuard failed to start",
            "no probe client",
        );
        assert_eq!(get_status().state, ShapingRuntimeState::Degraded);
        assert!(get_status().degraded);

        // The end-of-startup success announcement must NOT clobber the failure.
        mark_active("LibreQoS shaping is active.");
        let after_active = get_status();
        assert_eq!(
            after_active.state,
            ShapingRuntimeState::Degraded,
            "mark_active must not clobber a recorded failure"
        );
        assert!(after_active.degraded);
        assert_eq!(after_active.summary, "StormGuard failed to start");

        // An explicit success clears the latched failure.
        mark_recovered("LibreQoS shaping recovered.");
        let recovered = get_status();
        assert_eq!(recovered.state, ShapingRuntimeState::Active);
        assert!(!recovered.degraded);
        assert!(!recovered.can_retry);

        reset();
    }

    #[test]
    fn untainted_startup_reports_active() {
        let _guard = failure_surfacing_test_lock()
            .lock()
            .expect("failure-surfacing test lock should not be poisoned");
        reset();
        mark_starting("starting");
        mark_active("active");
        let status = get_status();
        assert_eq!(status.state, ShapingRuntimeState::Active);
        assert!(!status.degraded);
        reset();
    }
}
