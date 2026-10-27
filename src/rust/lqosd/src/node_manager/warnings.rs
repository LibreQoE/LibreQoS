//! A place to store warnings from system checks, the need to reload LibreQoS and similar.

use parking_lot::Mutex;
use serde::Serialize;

/// The maximum number of distinct operator warnings retained.
///
/// The sink is read on every API call (`get_global_warnings` clones the whole
/// list) and a repeating failure can be pushed once per retry/tick. Capping the
/// list (oldest evicted) keeps both the daemon's memory and the API payload
/// bounded. Repeated `(level, summary)` pairs are de-duplicated into a single
/// entry carrying a count, so a hot retry loop costs one slot, not one per try.
pub const MAX_GLOBAL_WARNINGS: usize = 64;

// `Info` is retained as a serialized wire variant (the warning UI switches on
// the `Info`/`Warning`/`Error` names); it is intentionally not constructed yet.
#[allow(dead_code)]
#[derive(Serialize, Debug, Copy, Clone, PartialEq, Eq)]
pub enum WarningLevel {
    Info,
    Warning,
    Error,
}

/// One de-duplicated operator warning.
#[derive(Debug, Clone)]
struct WarningEntry {
    level: WarningLevel,
    summary: String,
    count: u64,
}

static LQOSD_WARNINGS: Mutex<Vec<WarningEntry>> = Mutex::new(Vec::new());

/// Records an operator-visible warning.
///
/// A repeated `(level, summary)` pair increments the existing entry's count
/// instead of appending; at most [`MAX_GLOBAL_WARNINGS`] distinct entries are
/// retained, evicting the oldest first.
pub fn add_global_warning(level: WarningLevel, warning: String) {
    let mut warnings = LQOSD_WARNINGS.lock();
    if let Some(existing) = warnings
        .iter_mut()
        .find(|entry| entry.level == level && entry.summary == warning)
    {
        existing.count = existing.count.saturating_add(1);
        return;
    }
    warnings.push(WarningEntry {
        level,
        summary: warning,
        count: 1,
    });
    if warnings.len() > MAX_GLOBAL_WARNINGS {
        let overflow = warnings.len() - MAX_GLOBAL_WARNINGS;
        warnings.drain(0..overflow);
    }
}

/// Returns the operator warnings in insertion order.
///
/// The wire shape stays a `(level, message)` list; a de-duplicated entry carries
/// its repeat count as a `(xN)` suffix on the message so the count survives the
/// existing API contract.
pub fn get_global_warnings() -> Vec<(WarningLevel, String)> {
    LQOSD_WARNINGS
        .lock()
        .iter()
        .map(|entry| {
            let message = if entry.count > 1 {
                format!("{} (x{})", entry.summary, entry.count)
            } else {
                entry.summary.clone()
            };
            (entry.level, message)
        })
        .collect()
}

/// Test-only: clears the sink so tests do not leak process-global warning state.
#[cfg(test)]
pub fn clear_global_warnings() {
    LQOSD_WARNINGS.lock().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::failure_surfacing_test_lock;

    #[test]
    fn warnings_dedupe_and_stay_bounded() {
        let _guard = failure_surfacing_test_lock()
            .lock()
            .expect("failure-surfacing test lock should not be poisoned");
        clear_global_warnings();

        // A hot retry loop collapses to one entry with a count.
        for _ in 0..10 {
            add_global_warning(WarningLevel::Error, "bridge publish rejected".to_string());
        }
        let warnings = get_global_warnings();
        assert_eq!(warnings.len(), 1, "repeated failures must de-duplicate");
        assert_eq!(warnings[0].0, WarningLevel::Error);
        assert_eq!(warnings[0].1, "bridge publish rejected (x10)");

        // Distinct failures stay bounded (no unbounded growth / API payload).
        for i in 0..(MAX_GLOBAL_WARNINGS * 4) {
            add_global_warning(WarningLevel::Warning, format!("subsystem failure {i}"));
        }
        let bounded = get_global_warnings();
        assert!(
            bounded.len() <= MAX_GLOBAL_WARNINGS,
            "warning sink grew past the cap: {}",
            bounded.len()
        );
        // Oldest evicted, newest retained.
        let newest = format!("subsystem failure {}", MAX_GLOBAL_WARNINGS * 4 - 1);
        assert_eq!(
            bounded.last().map(|(_, message)| message.as_str()),
            Some(newest.as_str())
        );

        clear_global_warnings();
    }
}
