use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use lqos_bus::{UrgentIssue, UrgentSeverity, UrgentSource};
use parking_lot::Mutex;

use lqos_utils::unix_time::unix_now;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
static URGENT: Mutex<VecDeque<UrgentIssue>> = Mutex::new(VecDeque::new());

/// Serialises tests that read or mutate the process-global registry.
#[cfg(test)]
static URGENT_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Takes the registry test lock.
///
/// The registry outlives every test, so a test that submits an issue and then
/// counts it must not race a test that calls [`clear_all`].
#[cfg(test)]
pub(crate) fn test_lock() -> parking_lot::MutexGuard<'static, ()> {
    URGENT_TEST_LOCK.lock()
}

const MAX_ISSUES: usize = 100;
const TTL_SECONDS: u64 = 24 * 60 * 60; // 24h
const DEDUPE_WINDOW_SECONDS: u64 = 300; // 5 minutes

fn now_unix() -> u64 {
    unix_now().unwrap_or_default()
}

fn urgent_identity_key(code: &str, dedupe_key: Option<&str>) -> String {
    dedupe_key.unwrap_or(code).to_string()
}

fn prune_expired(q: &mut VecDeque<UrgentIssue>) {
    let now = now_unix();
    while let Some(front) = q.front() {
        if front.ts + TTL_SECONDS < now {
            q.pop_front();
        } else {
            break;
        }
    }
    while q.len() > MAX_ISSUES {
        q.pop_front();
    }
}

pub fn submit(
    source: UrgentSource,
    severity: UrgentSeverity,
    code: String,
    message: String,
    context: Option<String>,
    dedupe_key: Option<String>,
) {
    let ts = now_unix();
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed) + 1;
    let mut guard = URGENT.lock();
    prune_expired(&mut guard);

    // Dedupe: same (code, dedupe_key) within window updates timestamp/message.
    let key = urgent_identity_key(&code, dedupe_key.as_deref());
    if let Some(existing) = guard.iter_mut().rev().find(|i| {
        (i.code == code)
            && (i.dedupe_key.as_deref().unwrap_or("") == key)
            && (ts.saturating_sub(i.ts) <= DEDUPE_WINDOW_SECONDS)
    }) {
        existing.ts = ts;
        existing.message = message;
        existing.context = context;
        return;
    }

    let issue = UrgentIssue {
        id,
        ts,
        source,
        severity,
        code,
        message,
        context,
        dedupe_key: Some(key),
    };
    guard.push_back(issue);
    prune_expired(&mut guard);
}

pub fn list() -> Vec<UrgentIssue> {
    let mut guard = URGENT.lock();
    prune_expired(&mut guard);
    let mut v: Vec<UrgentIssue> = guard.iter().cloned().collect();
    v.sort_by_key(|i| i.ts);
    v.reverse();
    v
}

pub fn clear(id: u64) -> bool {
    let mut guard = URGENT.lock();
    if let Some(pos) = guard.iter().position(|i| i.id == id) {
        guard.remove(pos);
        true
    } else {
        false
    }
}

/// Clears urgent issues matching the same code and dedupe key used for submission.
pub fn clear_by_identity(code: &str, dedupe_key: &str) -> usize {
    let key = urgent_identity_key(code, Some(dedupe_key));
    let mut guard = URGENT.lock();
    let before = guard.len();
    guard.retain(|issue| {
        issue.code != code || issue.dedupe_key.as_deref().unwrap_or("") != key.as_str()
    });
    before.saturating_sub(guard.len())
}

/// The code a recovered issue is retained under (see
/// [`mark_recovered_by_identity`]).
pub fn recovered_code(code: &str) -> String {
    format!("{code}_RECOVERED")
}

/// Heals the ACTIVE issues of `(code, dedupe_key)` WITHOUT hiding the event:
/// they are replaced by ONE retained `<code>_RECOVERED` Warning record, so an
/// operator who opens the list after the heal still sees it (jackrabbit
/// 2026-11-16: alerts healed between 30 s polls and were never seen,
/// `lqos-74t.197`).
///
/// - An earlier recovered record of the same identity is replaced, so there is
///   one per identity.
/// - The record goes like any other item: operator acknowledge, or the 24 h TTL.
/// - A later re-raise of `code` is a NEW active item, because the recovered
///   code never dedupes with it.
///
/// Returns the number of active issues converted; 0 means nothing was recorded.
pub fn mark_recovered_by_identity(
    code: &str,
    dedupe_key: &str,
    message: String,
    context: Option<String>,
) -> usize {
    let key = urgent_identity_key(code, Some(dedupe_key));
    let recovered = recovered_code(code);
    let mut guard = URGENT.lock();
    prune_expired(&mut guard);
    let same_key = |issue: &UrgentIssue| issue.dedupe_key.as_deref().unwrap_or("") == key;
    let active = guard.iter().filter(|i| i.code == code && same_key(i)).count();
    let Some(source) = guard
        .iter()
        .find(|i| i.code == code && same_key(i))
        .map(|i| i.source.clone())
    else {
        return 0;
    };
    guard.retain(|i| !(same_key(i) && (i.code == code || i.code == recovered)));
    guard.push_back(UrgentIssue {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed) + 1,
        ts: now_unix(),
        source,
        severity: UrgentSeverity::Warning,
        code: recovered,
        message,
        context,
        dedupe_key: Some(key),
    });
    prune_expired(&mut guard);
    active
}

pub fn clear_all() {
    let mut guard = URGENT.lock();
    guard.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_by_identity_removes_only_matching_issues() {
        let _guard = test_lock();
        let code = "TEST_XDP_IP_MAPPING_APPLY_FAILED";
        let other_code = "TEST_OTHER";
        let other_dedupe = "TEST_XDP_IP_MAPPING_APPLY_FAILED_OTHER_DEDUPE";
        clear_by_identity(code, code);
        clear_by_identity(code, other_dedupe);
        clear_by_identity(other_code, code);
        submit(
            UrgentSource::LibreQoS,
            UrgentSeverity::Error,
            code.to_string(),
            "mapping failed".to_string(),
            None,
            Some(code.to_string()),
        );
        submit(
            UrgentSource::LibreQoS,
            UrgentSeverity::Error,
            code.to_string(),
            "same code different dedupe".to_string(),
            None,
            Some(other_dedupe.to_string()),
        );
        submit(
            UrgentSource::LibreQoS,
            UrgentSeverity::Error,
            other_code.to_string(),
            "other failed".to_string(),
            None,
            Some(code.to_string()),
        );

        let cleared = clear_by_identity(code, code);
        let matching_issue_survived = list()
            .into_iter()
            .any(|issue| issue.code == code && issue.dedupe_key.as_deref() == Some(other_dedupe));

        assert_eq!(cleared, 1);
        assert!(matching_issue_survived);
        assert_eq!(clear_by_identity(code, other_dedupe), 1);
        assert_eq!(clear_by_identity(other_code, code), 1);
    }

    #[test]
    fn seam_replaced_urgent_clears_on_heal() {
        let _guard = test_lock();
        clear_all();
        submit(
            UrgentSource::System,
            UrgentSeverity::Warning,
            "BRIDGE_SEAM_REPLACED".to_string(),
            "seam dead".to_string(),
            None,
            Some("bridge-broker:seam-replaced".to_string()),
        );
        assert!(list().iter().any(|i| i.code == "BRIDGE_SEAM_REPLACED"));
        // The heal path (a generation APPLIED after re-attach) clears it.
        assert_eq!(
            clear_by_identity("BRIDGE_SEAM_REPLACED", "bridge-broker:seam-replaced"),
            1
        );
        assert!(list().iter().all(|i| i.code != "BRIDGE_SEAM_REPLACED"));
    }

    #[test]
    fn recovered_issue_stays_visible_and_a_reraise_is_a_new_active_item() {
        let _guard = test_lock();
        clear_all();
        let (code, key) = ("CIRCUIT_BLACKHOLE", "circuit-blackhole:x");
        // Healing an identity that was never raised records nothing.
        assert_eq!(mark_recovered_by_identity(code, key, "r".into(), None), 0);
        assert!(list().is_empty());

        submit(
            UrgentSource::System,
            UrgentSeverity::Error,
            code.to_string(),
            "held".to_string(),
            Some("ctx".to_string()),
            Some(key.to_string()),
        );
        assert_eq!(
            mark_recovered_by_identity(code, key, "recovered".into(), Some("raised_at_unix=1".into())),
            1
        );
        let items = list();
        assert_eq!(items.len(), 1, "the event stays visible (badge count 1)");
        assert_eq!(items[0].code, "CIRCUIT_BLACKHOLE_RECOVERED");
        assert_eq!(items[0].severity, UrgentSeverity::Warning);
        assert_eq!(items[0].context.as_deref(), Some("raised_at_unix=1"));

        // A re-raise is a NEW active Error item beside the recovered record.
        submit(
            UrgentSource::System,
            UrgentSeverity::Error,
            code.to_string(),
            "held again".to_string(),
            None,
            Some(key.to_string()),
        );
        let items = list();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.code == code && i.severity == UrgentSeverity::Error));
        // Its heal replaces the old recovered record: one per identity.
        assert_eq!(mark_recovered_by_identity(code, key, "recovered 2".into(), None), 1);
        let items = list();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].message, "recovered 2");
        // The recovered record is operator-clearable.
        assert!(clear(items[0].id));
        assert!(list().is_empty());
    }

    #[test]
    fn urgent_identity_key_falls_back_to_code() {
        assert_eq!(
            urgent_identity_key("CODE_ONLY", None),
            "CODE_ONLY".to_string()
        );
        assert_eq!(
            urgent_identity_key("CODE", Some("DEDUP")),
            "DEDUP".to_string()
        );
    }
}
