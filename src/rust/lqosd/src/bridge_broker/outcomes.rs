//! Operator-visible publish outcomes (E3b).
//!
//! A bounded, read-only projection of the broker actor's own
//! transitions — the same facts `ack_map` classifies and the actor
//! already logs, retained so the UI can show the pending → settled
//! lifecycle of a config change. NO new writer, NO new seam traffic:
//! the actor records what it was going to do anyway, the read side
//! never mutates.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Settled-outcome ring bound. Small on purpose: this is an
/// operator's "what happened to my last changes" surface, not a log.
const SETTLED_RING_MAX: usize = 16;

/// Lifecycle state of one published generation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishState {
    /// Emitted, waiting for the bridge ack. `seq_pos`/`seq_len`
    /// locate the generation inside its compliance sequence (1-based).
    Pending { seq_pos: usize, seq_len: usize },
    /// `ATTEMPT_EXHAUSTED` retry in progress (the only bounded-retry
    /// outcome).
    Retrying { attempt: u32, max: u32 },
    /// The bridge acked APPLIED.
    Applied,
    /// Terminal for this generation: typed reason surfaced verbatim
    /// (REJECTED_*, ACK_SILENCE, TOO_LARGE_FOR_SEAM, OBSERVE_MODE).
    Rejected { reason: String },
}

/// One generation's operator-visible publish record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublishRecord {
    pub generation: String,
    pub state: PublishState,
    pub updated_unix: u64,
}

#[derive(Default)]
struct OutcomeLog {
    /// The in-flight generation, if any (never duplicated into the
    /// ring until it settles).
    pending: Option<PublishRecord>,
    /// Settled records, oldest first.
    settled: std::collections::VecDeque<PublishRecord>,
}

static LOG: OnceLock<Mutex<OutcomeLog>> = OnceLock::new();

fn log() -> &'static Mutex<OutcomeLog> {
    LOG.get_or_init(|| Mutex::new(OutcomeLog::default()))
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The actor emitted a generation (or advanced its sequence position).
pub fn record_emit(generation: &str, seq_pos: usize, seq_len: usize) {
    let mut log = log().lock();
    let record = PublishRecord {
        generation: generation.to_string(),
        state: PublishState::Pending { seq_pos, seq_len },
        updated_unix: now_unix(),
    };
    log.pending = Some(record);
}

/// The actor is in a bounded `ATTEMPT_EXHAUSTED` retry.
pub fn record_retry(generation: &str, attempt: u32, max: u32) {
    let mut log = log().lock();
    let record = PublishRecord {
        generation: generation.to_string(),
        state: PublishState::Retrying { attempt, max },
        updated_unix: now_unix(),
    };
    log.pending = Some(record);
}

/// The generation settled (either arm). Ring-bounded; the oldest
/// settled record falls off.
fn settle(generation: &str, state: PublishState) {
    let mut log = log().lock();
    if log.pending.as_ref().is_some_and(|p| p.generation == generation) {
        log.pending = None;
    }
    let record = PublishRecord {
        generation: generation.to_string(),
        state,
        updated_unix: now_unix(),
    };
    if log.settled.len() >= SETTLED_RING_MAX {
        log.settled.pop_front();
    }
    log.settled.push_back(record);
}

/// The bridge acked APPLIED.
pub fn record_applied(generation: &str) {
    // F3-B §3b (required 8): a successful apply CLEARS prior `SEAM_LOST`
    // seam-transport records so `last_reject` means a real DATA PLANE reject,
    // not a bridge restart/segment loss. (`SEAM_LOST` stays visible during the
    // outage; the first re-adopt clears it.)
    {
        let mut log = log().lock();
        log.settled.retain(
            |r| !matches!(&r.state, PublishState::Rejected { reason } if reason == "SEAM_LOST"),
        );
    }
    settle(generation, PublishState::Applied);
}

/// Terminal reject — surfaced honestly, never retried.
pub fn record_rejected(generation: &str, reason: &str) {
    settle(
        generation,
        PublishState::Rejected {
            reason: reason.to_string(),
        },
    );
}

/// Snapshot for the read surface: the pending record (if any) first,
/// then settled records newest-first.
pub fn publish_outcomes() -> Vec<PublishRecord> {
    let log = log().lock();
    let mut out = Vec::with_capacity(1 + log.settled.len());
    if let Some(pending) = log.pending.as_ref() {
        out.push(pending.clone());
    }
    out.extend(log.settled.iter().rev().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use once_cell::sync::Lazy;

    /// The log is process-global: serialize the tests that touch it
    /// (cargo runs tests in one binary on parallel threads).
    static LOG_TEST_LOCK: Lazy<parking_lot::Mutex<()>> =
        Lazy::new(|| parking_lot::Mutex::new(()));

    /// Takes the log lock and empties it for one test.
    fn reset() -> parking_lot::MutexGuard<'static, ()> {
        let guard = LOG_TEST_LOCK.lock();
        *log().lock() = OutcomeLog::default();
        guard
    }

    #[test]
    fn emit_sets_pending_with_sequence_position() {
        let _guard = reset();
        record_emit("gen-a", 1, 3);
        let outcomes = publish_outcomes();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].state,
            PublishState::Pending {
                seq_pos: 1,
                seq_len: 3
            }
        );
    }

    #[test]
    fn sequence_advance_updates_position_without_duplicating() {
        let _guard = reset();
        record_emit("gen-a", 1, 3);
        record_emit("gen-a", 2, 3);
        let outcomes = publish_outcomes();
        assert_eq!(outcomes.len(), 1, "same generation upserts, never duplicates");
        assert_eq!(
            outcomes[0].state,
            PublishState::Pending {
                seq_pos: 2,
                seq_len: 3
            }
        );
    }

    #[test]
    fn applied_settles_and_clears_pending() {
        let _guard = reset();
        record_emit("gen-a", 1, 2);
        record_applied("gen-a");
        let outcomes = publish_outcomes();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].state, PublishState::Applied);
        record_emit("gen-b", 1, 1);
        let outcomes = publish_outcomes();
        assert_eq!(outcomes.len(), 2, "pending first, settled after");
        assert_eq!(outcomes[0].generation, "gen-b");
        assert_eq!(outcomes[1].generation, "gen-a");
    }

    #[test]
    fn rejected_carries_the_typed_reason_verbatim() {
        let _guard = reset();
        record_emit("gen-a", 1, 1);
        record_rejected("gen-a", "REJECTED_OBSERVE_MODE");
        let outcomes = publish_outcomes();
        assert_eq!(
            outcomes[0].state,
            PublishState::Rejected {
                reason: "REJECTED_OBSERVE_MODE".to_string()
            }
        );
    }

    #[test]
    fn retry_state_is_visible() {
        let _guard = reset();
        record_emit("gen-a", 1, 1);
        record_retry("gen-a", 1, 3);
        let outcomes = publish_outcomes();
        assert_eq!(
            outcomes[0].state,
            PublishState::Retrying { attempt: 1, max: 3 }
        );
    }

    #[test]
    fn settled_ring_is_bounded() {
        let _guard = reset();
        for i in 0..(SETTLED_RING_MAX + 4) {
            record_applied(&format!("gen-{i}"));
        }
        let outcomes = publish_outcomes();
        assert_eq!(outcomes.len(), SETTLED_RING_MAX);
        assert_eq!(outcomes[0].generation, format!("gen-{}", SETTLED_RING_MAX + 3), "newest first");
    }

    #[test]
    fn unknown_generation_settle_still_lands_in_the_ring() {
        let _guard = reset();
        record_emit("gen-a", 1, 1);
        // A replace-mid-publish clears pending; a late ack for a
        // different generation must still surface, never panic.
        record_rejected("gen-x", "ACK_SILENCE");
        let outcomes = publish_outcomes();
        assert_eq!(outcomes.len(), 2, "pending gen-a + settled gen-x");
        assert_eq!(outcomes[1].state, PublishState::Rejected { reason: "ACK_SILENCE".to_string() });
    }

    /// `lqos-74t.32.1` (work-order REQUIRED 8, the "clear on re-adopt" arm):
    /// a seam-transport `SEAM_LOST` must NOT permanently share the
    /// `last_reject` slot with a dataplane reject. During the outage it IS the
    /// newest record (visible, the interim tolerance); the first successful
    /// re-adopt CLEARS it, revealing the dataplane reject underneath.
    #[test]
    fn seam_lost_clears_on_readopt_and_never_masks_a_dataplane_reject() {
        let _guard = reset();
        // 1. a dataplane/mechanics reject ...
        record_rejected("gen-dataplane", "REJECTED_STRUCTURAL_OP");
        // 2. ... then a seam-transport loss (newest: it OWNS last_reject during the outage).
        record_rejected("gen-seam", "SEAM_LOST");
        let outcomes = publish_outcomes();
        assert!(
            matches!(&outcomes[0].state, PublishState::Rejected { reason } if reason == "SEAM_LOST"),
            "during the outage the seam transient is the newest record"
        );
        // 3. the bridge re-attaches + applies: the SEAM_LOST clears.
        record_applied("gen-readopt");
        let outcomes = publish_outcomes();
        assert!(
            outcomes.iter().all(
                |r| !matches!(&r.state, PublishState::Rejected { reason } if reason == "SEAM_LOST")
            ),
            "SEAM_LOST must not survive a successful apply"
        );
        assert!(
            outcomes.iter().any(
                |r| matches!(&r.state, PublishState::Rejected { reason } if reason == "REJECTED_STRUCTURAL_OP")
            ),
            "the dataplane reject must remain visible after the seam transient clears"
        );
    }

    #[test]
    fn retained_generation_settles_a_new_record_never_dedupes() {
        // F-NA P2 (docs/false_no_ack_outcome_card.md): a return to a
        // retained generation (revert-to-plan) must surface as a NEW
        // outcome — the server side must never dedupe it away, or the
        // minted-generation poll would starve to the watchdog.
        let _guard = reset();
        record_applied("gen-base");
        record_applied("gen-override");

        // The revert publishes the base generation again: emit (pending)
        // first — the snapshot shows the pending record for gen-base
        // BEFORE its old applied record (newest wins, find-first).
        record_emit("gen-base", 1, 1);
        let outcomes = publish_outcomes();
        assert_eq!(outcomes[0].generation, "gen-base");
        assert_eq!(
            outcomes[0].state,
            PublishState::Pending { seq_pos: 1, seq_len: 1 },
            "the retained generation's NEW fence is visible while pending"
        );

        // The bridge acks — a SECOND settled record for gen-base exists
        // (ring-bounded, newest-first), never deduped into the old one.
        record_applied("gen-base");
        let outcomes = publish_outcomes();
        let base_records: Vec<&PublishRecord> =
            outcomes.iter().filter(|r| r.generation == "gen-base").collect();
        assert_eq!(base_records.len(), 2, "retained gen keeps both records");
        assert_eq!(outcomes[0].generation, "gen-base");
        assert_eq!(outcomes[0].state, PublishState::Applied,
            "find-first lands on the NEWEST record — the UI settles APPLIED");
        assert!(
            base_records[0].updated_unix >= base_records[1].updated_unix,
            "newest-first ordering holds for the duplicated generation"
        );
    }
}
