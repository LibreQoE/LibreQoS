//! A place to store warnings from system checks, the need to reload LibreQoS and similar.

use parking_lot::Mutex;
use serde::Serialize;

#[allow(dead_code)]
#[derive(Serialize, Debug, Copy, Clone)]
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
