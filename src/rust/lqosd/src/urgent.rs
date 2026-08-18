use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use lqos_bus::{UrgentIssue, UrgentSeverity, UrgentSource};
use parking_lot::Mutex;

use lqos_utils::unix_time::unix_now;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
static URGENT: Mutex<VecDeque<UrgentIssue>> = Mutex::new(VecDeque::new());

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

pub fn clear_all() {
    let mut guard = URGENT.lock();
    guard.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_by_identity_removes_only_matching_issues() {
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
