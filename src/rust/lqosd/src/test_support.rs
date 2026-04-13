use std::sync::{Mutex, OnceLock};

/// Serializes tests that mutate process-global runtime configuration state.
pub(crate) fn runtime_config_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
