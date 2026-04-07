use std::sync::atomic::{AtomicU64, Ordering};

use lqos_bus::SchedulerProgressReport;
use lqos_utils::unix_time::unix_now;
use parking_lot::Mutex;

static API_LAST_SEEN: AtomicU64 = AtomicU64::new(0);
static CHATBOT_LAST_SEEN: AtomicU64 = AtomicU64::new(0);
static SCHEDULER_LAST_SEEN: AtomicU64 = AtomicU64::new(0);
static SCHEDULER_ERROR: Mutex<Option<String>> = Mutex::new(None);
static SCHEDULER_OUTPUT: Mutex<Option<String>> = Mutex::new(None);
static SCHEDULER_PROGRESS: Mutex<Option<SchedulerProgressReport>> = Mutex::new(None);

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

/// Sets the latest scheduler output message.
///
/// # Arguments
/// * `output` - An optional string containing informational scheduler output
pub fn scheduler_output(output: Option<String>) {
    let mut guard = SCHEDULER_OUTPUT.lock();
    *guard = output;
}

/// Sets the latest scheduler progress state.
pub fn scheduler_progress(progress: Option<SchedulerProgressReport>) {
    let mut guard = SCHEDULER_PROGRESS.lock();
    *guard = progress;
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

/// Returns the current scheduler output message, if any.
pub fn scheduler_output_message() -> Option<String> {
    let guard: parking_lot::lock_api::MutexGuard<'_, parking_lot::RawMutex, Option<String>> =
        SCHEDULER_OUTPUT.lock();
    guard.clone()
}

/// Returns the current scheduler progress state, if any.
pub fn scheduler_progress_state() -> Option<SchedulerProgressReport> {
    let guard: parking_lot::lock_api::MutexGuard<
        '_,
        parking_lot::RawMutex,
        Option<SchedulerProgressReport>,
    > = SCHEDULER_PROGRESS.lock();
    guard.clone()
}
