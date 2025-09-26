use std::sync::atomic::{AtomicU64, Ordering};

use lqos_utils::unix_time::unix_now;
use parking_lot::Mutex;

static API_LAST_SEEN: AtomicU64 = AtomicU64::new(0);
static CHATBOT_LAST_SEEN: AtomicU64 = AtomicU64::new(0);
static SCHEDULER_LAST_SEEN: AtomicU64 = AtomicU64::new(0);
static SCHEDULER_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Updates the last seen timestamp for the API.
pub fn api_seen() {
    let Ok(now) = unix_now() else {
        return;
    };
    API_LAST_SEEN.store(now, Ordering::Relaxed);
}

/// Updates the last seen timestamp for the ChatBot.
pub fn chatbot_seen() {
    let Ok(now) = unix_now() else {
        return;
    };
    CHATBOT_LAST_SEEN.store(now, Ordering::Relaxed);
}

/// Updates the last seen timestamp for the Scheduler.
pub fn scheduler_seen() {
    let Ok(now) = unix_now() else {
        return;
    };
    SCHEDULER_LAST_SEEN.store(now, Ordering::Relaxed);
}

/// Sets the scheduler error message.
///
/// # Arguments
/// * `err` - An optional string containing the error message
pub fn scheduler_error(err: Option<String>) {
    let mut guard = SCHEDULER_ERROR.lock();
    *guard = err;
}

/// Checks if the API is available.
///
/// Returns `true` if the API has been seen within the last 5 minutes.
pub fn is_api_available() -> bool {
    // If the API has called in within the last 5 minutes, consider it available
    let Ok(now) = unix_now() else {
        return false;
    };
    let last = API_LAST_SEEN.load(Ordering::Relaxed);
    now.saturating_sub(last) < 300
}

/// Checks if the ChatBot is available.
///
/// Returns `true` if the ChatBot has been seen within the last 5 minutes.
pub fn is_chatbot_available() -> bool {
    // If the ChatBot has called in within the last 5 minutes, consider it available
    let Ok(now) = unix_now() else {
        return false;
    };
    let last = CHATBOT_LAST_SEEN.load(Ordering::Relaxed);
    now.saturating_sub(last) < 300
}

/// Checks if the Scheduler is available.
///
/// Returns `true` if the Scheduler has been seen within the last 5 minutes.
pub fn is_scheduler_available() -> bool {
    // If the Scheduler has called in within the last 5 minutes, consider it available
    let Ok(now) = unix_now() else {
        return false;
    };
    let last = SCHEDULER_LAST_SEEN.load(Ordering::Relaxed);
    now.saturating_sub(last) < 300
}

/// Returns the current scheduler error message, if any.
pub fn scheduler_error_message() -> Option<String> {
    let guard: parking_lot::lock_api::MutexGuard<'_, parking_lot::RawMutex, Option<String>> =
        SCHEDULER_ERROR.lock();
    guard.clone()
}
