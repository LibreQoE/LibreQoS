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
    if let Ok(n) = unix_now() { n } else { 0 }
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

    // Dedupe: same (code, dedupe_key) within window → update timestamp/message
    let key = dedupe_key.clone().unwrap_or_else(|| code.clone());
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

pub fn clear_all() {
    let mut guard = URGENT.lock();
    guard.clear();
}
