//! A place to store warnings from sanity checks, the need to reload LibreQoS and similar.

use serde::Serialize;
use parking_lot::Mutex;

#[allow(dead_code)]
#[derive(Serialize, Copy, Clone)]
pub enum WarningLevel {
    Info,
    Warning,
    Error,
}

static LQOSD_WARNINGS: Mutex<Vec<(WarningLevel, String)>> = Mutex::new(Vec::new());

pub fn add_global_warning(level: WarningLevel, warning: String) {
    LQOSD_WARNINGS.lock().push((level, warning));
}

pub fn get_global_warnings() -> Vec<(WarningLevel, String)> {
    LQOSD_WARNINGS.lock().clone()
}
