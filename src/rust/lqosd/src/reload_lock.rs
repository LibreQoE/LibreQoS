use std::sync::atomic::{AtomicBool, Ordering};

static RELOAD_BUSY: AtomicBool = AtomicBool::new(false);

pub(crate) struct ReloadGuard;

impl Drop for ReloadGuard {
    fn drop(&mut self) {
        RELOAD_BUSY.store(false, Ordering::Release);
    }
}

pub(crate) enum ReloadExecOutcome {
    Busy,
    Success(String),
    Failed(String),
}

fn try_acquire_reload_guard() -> Option<ReloadGuard> {
    RELOAD_BUSY
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .ok()
        .map(|_| ReloadGuard)
}

/// Attempts to reload LibreQoS, returning `Busy` if another reload is already running.
pub(crate) fn try_reload_libreqos_locked() -> ReloadExecOutcome {
    let Some(_guard) = try_acquire_reload_guard() else {
        return ReloadExecOutcome::Busy;
    };

    match lqos_config::load_libreqos() {
        Ok(message) => ReloadExecOutcome::Success(message),
        Err(e) => ReloadExecOutcome::Failed(e.to_string()),
    }
}
