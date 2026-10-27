//! Typed-ack → operator-visible error mapping (E1).
//!
//! Every reject class becomes an urgent issue (dedupe key = the
//! generation, so repeated observation of the same rejected config
//! collapses to one operator-visible item). The ONLY outcome eligible
//! for bounded auto-retry is `ATTEMPT_EXHAUSTED` (its documented
//! semantics are "not applied; retry" — fresh-#12 G5); everything
//! else is terminal for that generation, and the broker refuses to
//! re-emit a terminally-failed generation until the config changes
//! (the retry-storm fence).

use crate::urgent;
use lqos_bus::{UrgentSeverity, UrgentSource};
use lqos_dataplane_types::wire::AckOutcome;
use std::sync::atomic::{AtomicU64, Ordering};

/// C2 churn tripwire counters (cold-path): cumulative APPLIED acks and
/// non-applied (reject/retry) outcomes since process start. The outage
/// monitor diffs them over a window.
static APPLIED_EVENTS: AtomicU64 = AtomicU64::new(0);
static REJECT_EVENTS: AtomicU64 = AtomicU64::new(0);

/// Records one APPLIED ack (a generation reached the bridge).
pub(crate) fn record_applied() {
    APPLIED_EVENTS.fetch_add(1, Ordering::Relaxed);
}

/// `(applied, rejected)` cumulative counters for the churn tripwire.
pub(crate) fn churn_counters() -> (u64, u64) {
    (
        APPLIED_EVENTS.load(Ordering::Relaxed),
        REJECT_EVENTS.load(Ordering::Relaxed),
    )
}

/// The broker-level classification of one ack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublishOutcome {
    /// Applied; the acked generation becomes the new baseline.
    Applied,
    /// "Not applied; dataplane unchanged; retry" — the only outcome
    /// eligible for bounded auto-retry.
    AttemptExhausted,
    /// The bridge runs `queue_mode=observe` — surface honestly, never
    /// retry (config change or bridge restart is the operator path).
    ObserveMode,
    /// F3-B §3b: the delta is not delta-expressible (a `StructurallyInvalid`
    /// class) OR an owner-set/span mechanics violation — the bridge takes its
    /// cold, content-keyed REBUILD. The broker must NOT quarantine
    /// (`held_ops_from_tx`), arm `HELD_RETRY`, or set `terminal_fence`; it waits
    /// for the post-rebuild report and re-diffs.
    RebuildRequired,
    /// Every other typed reject: terminal for this generation.
    Rejected(&'static str),
}

/// U1/W1: the broker classifies the SHARED `AckOutcome` vocabulary (one
/// definition with both ends) — never a hand-copied numeric match.
pub(crate) fn classify(outcome: u8) -> PublishOutcome {
    match AckOutcome::from_code(outcome) {
        Some(AckOutcome::Applied) => PublishOutcome::Applied,
        Some(AckOutcome::AttemptExhausted) => PublishOutcome::AttemptExhausted,
        Some(AckOutcome::RejectedObserveMode) => PublishOutcome::ObserveMode,
        Some(AckOutcome::RebuildRequired) => PublishOutcome::RebuildRequired,
        Some(AckOutcome::RejectedInvalidFrame) => {
            PublishOutcome::Rejected("REJECTED_INVALID_FRAME")
        }
        Some(AckOutcome::RejectedMalformedJson) => {
            PublishOutcome::Rejected("REJECTED_MALFORMED_JSON")
        }
        Some(AckOutcome::RejectedReservedRoot) => {
            PublishOutcome::Rejected("REJECTED_RESERVED_ROOT")
        }
        Some(AckOutcome::RejectedStructuralOp) => {
            PublishOutcome::Rejected("REJECTED_STRUCTURAL_OP")
        }
        Some(AckOutcome::RejectedLpmPrepare) => PublishOutcome::Rejected("REJECTED_LPM_PREPARE"),
        Some(AckOutcome::RejectedVersionMismatch) => {
            PublishOutcome::Rejected("REJECTED_VERSION_MISMATCH")
        }
        Some(AckOutcome::RejectedBatchEnvelope) => {
            PublishOutcome::Rejected("REJECTED_BATCH_ENVELOPE")
        }
        None => PublishOutcome::Rejected("UNKNOWN_OUTCOME"),
    }
}

/// The stable dedupe identity for an operator-visible reject.
///
/// A STRUCTURAL reject is a property of the WORLD's structure, not the
/// (rate-churning) shaping generation, so it keys on ONE stable identity:
/// repeated refusals collapse to a single operator item and the apply path can
/// clear it on heal. A REBUILD_REQUIRED is likewise a property of the world
/// (what the bridge must cold-rebuild), so it too keys on ONE stable identity —
/// otherwise the per-generation key accumulates one item per rebuild and the
/// apply path cannot clear them. Every other reject keys on its generation.
pub(crate) fn reject_dedupe_key(outcome: PublishOutcome, generation: &str) -> String {
    match outcome {
        PublishOutcome::Rejected("REJECTED_STRUCTURAL_OP") => {
            "bridge-broker:structural".to_string()
        }
        PublishOutcome::RebuildRequired => "bridge-broker:rebuild".to_string(),
        _ => format!("bridge-broker:{generation}"),
    }
}

/// Surfaces one non-applied outcome as an urgent issue (deduped by
/// generation). APPLIED is not routed here — the actor records it.
///
/// U1/W4: the bridge's TYPED reject detail (reason + identity) is now carried
/// through to the operator copy — the code-only ack was the missing input that
/// cost a whole session.
/// `lqos-zxc.6`: render the ack's declared drops for the operator copy.
/// `ACK_DROPS_UNKNOWN` ⇒ "predicted (unmeasured)" (never a silent 0); a real
/// count ⇒ "N (predicted|observed)".
pub(crate) fn drops_text(
    drops: Option<(lqos_dataplane_types::wire::AckDropsKind, u32)>,
) -> String {
    use lqos_dataplane_types::wire::{ACK_DROPS_UNKNOWN, AckDropsKind};
    drops
        .map(|(kind, packets)| {
            let label = match kind {
                AckDropsKind::Predicted => "predicted",
                AckDropsKind::Observed => "observed",
                AckDropsKind::None => "none",
            };
            if packets == ACK_DROPS_UNKNOWN {
                format!(" drops: {label} (unmeasured)")
            } else {
                format!(" drops: {packets} ({label})")
            }
        })
        .unwrap_or_default()
}

pub(crate) fn surface(
    outcome: PublishOutcome,
    generation: &str,
    fence_epoch: u64,
    reason: Option<lqos_dataplane_types::wire::RejectReason>,
    identity: &str,
    drops: Option<(lqos_dataplane_types::wire::AckDropsKind, u32)>,
) {
    let reason_txt = reason
        .map(|r| format!("{r:?}"))
        .unwrap_or_else(|| "unknown".to_string());
    let id_txt = if identity.is_empty() {
        String::new()
    } else {
        format!(" at {identity}")
    };
    // `lqos-zxc.6`: the ack DECLARES the drop source/timing. `ACK_DROPS_UNKNOWN`
    // renders as "predicted (unmeasured)" (PIN-1: never a silent 0); a real count
    // renders as "N (predicted|observed)".
    let drops_txt = drops_text(drops);
    let (code, severity, message) = match outcome {
        PublishOutcome::Applied => return,
        PublishOutcome::AttemptExhausted => (
            "BRIDGE_PUBLISH_ATTEMPT_EXHAUSTED",
            UrgentSeverity::Warning,
            format!(
                "Bridge publish of generation {generation} exhausted its install attempts \
                 (not applied; dataplane unchanged). Retrying; if this persists, check \
                 bridge load and the bridge log."
            ),
        ),
        PublishOutcome::ObserveMode => (
            "BRIDGE_PUBLISH_REJECTED_OBSERVE_MODE",
            UrgentSeverity::Warning,
            format!(
                "Bridge is running queue_mode=observe (the on/off A/B posture) and \
                 rejected generation {generation}: no shaping state exists to apply. \
                 Restart the bridge in shape mode to re-shape; this is not retried \
                 automatically."
            ),
        ),
        PublishOutcome::Rejected(name) => (
            "BRIDGE_PUBLISH_REJECTED",
            UrgentSeverity::Error,
            if name == "REJECTED_STRUCTURAL_OP" {
                // L2 (`docs/control_plane_structural_convergence_card.md` §3): the
                // world-freeze is GONE. A structural reject quarantines the one
                // offending op; the rest of the world keeps applying. U1/W4: the
                // typed reason + the offending identity name the op exactly.
                // (Supersedes the R3 CAUSE-NEUTRAL copy: that was written when the
                // ack carried NO reason; U1 now CARRIES the typed reason, so the
 // copy is cause-ACCURATE — the implementer ``/``.)
                format!(
                    "Bridge refused generation {generation} with {name} [{reason_txt}{id_txt}]: \
                     one site-layer change is not appliable yet (for example a multi-add, a \
                     span-changing re-parent, or a retire whose circuits have not drained). \
                     That single change is HELD (quarantined); rates and every unaffected \
                     structural change keep applying, so no restart is needed. The held change \
                     converges automatically once its cause clears (or on a bridge re-attach) — \
                     see the Bridge page for the held op."
                )
            } else {
                format!(
                    "Bridge rejected generation {generation} with {name} [{reason_txt}{id_txt}] \
                     (fence epoch {fence_epoch}). The dataplane keeps the last acknowledged \
                     state. Fix the reported cause; the next config change re-emits."
                )
            },
        ),
        PublishOutcome::RebuildRequired => (
            "BRIDGE_REBUILD_REQUIRED",
            UrgentSeverity::Warning,
            format!(
                "Bridge reports generation {generation} needs a COLD REBUILD \
                 [{reason_txt}{id_txt}]: the change is not delta-expressible (a \
                 structural-invalid class) or it changes an owner set/span. The bridge is \
                 rebuilding from the compiled desired — atomic at the fence, no live owner \
                 change. The broker is NOT holding or fencing it; the world converges on the \
                 post-rebuild report.{}",
                drops_txt
            ),
        ),
    };
    // A STRUCTURAL reject keys on ONE stable identity (see
    // `reject_dedupe_key`), so repeated refusals collapse to one operator item
    // and the apply path can clear it on heal (`clear_by_identity`).
    REJECT_EVENTS.fetch_add(1, Ordering::Relaxed);
    let dedupe = reject_dedupe_key(outcome, generation);
    urgent::submit(
        UrgentSource::System,
        severity,
        code.to_string(),
        message,
        Some(format!("generation={generation}")),
        Some(dedupe),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_codes_map_to_classes() {
        assert_eq!(classify(1), PublishOutcome::Applied);
        assert_eq!(classify(7), PublishOutcome::AttemptExhausted);
        assert_eq!(classify(10), PublishOutcome::ObserveMode);
        assert!(matches!(
            classify(5),
            PublishOutcome::Rejected("REJECTED_STRUCTURAL_OP")
        ));
        assert!(matches!(
            classify(99),
            PublishOutcome::Rejected("UNKNOWN_OUTCOME")
        ));
    }

    #[test]
    fn only_attempt_exhausted_is_retry_eligible() {
        for code in [2u8, 3, 4, 5, 6, 8, 9, 10, 99] {
            assert_ne!(classify(code), PublishOutcome::AttemptExhausted);
        }
    }

    #[test]
    fn structural_rejects_share_one_stable_dedupe_key() {
        let a = reject_dedupe_key(PublishOutcome::Rejected("REJECTED_STRUCTURAL_OP"), "gen-A");
        let b = reject_dedupe_key(PublishOutcome::Rejected("REJECTED_STRUCTURAL_OP"), "gen-B");
        assert_eq!(a, b, "structural rejects collapse to one identity");
        let c = reject_dedupe_key(PublishOutcome::Rejected("REJECTED_LPM_PREPARE"), "gen-A");
        assert_ne!(a, c, "a non-structural reject keys on its generation");
        assert!(c.contains("gen-A"));
    }

    /// The urgent layer (fresh-#12 required): two structural refusals on
    /// different generations are ONE operator item, and the heal clears it by
    /// `lqos-zxc.6`: the drops render — UNKNOWN ⇒ "predicted (unmeasured)"
    /// (never a silent 0); a real count ⇒ "N (predicted)"; absent ⇒ "".
    #[test]
    fn drops_text_renders_the_declared_source_and_count() {
        use lqos_dataplane_types::wire::{ACK_DROPS_UNKNOWN, AckDropsKind};
        assert_eq!(
            drops_text(Some((AckDropsKind::Predicted, ACK_DROPS_UNKNOWN))),
            " drops: predicted (unmeasured)"
        );
        assert_eq!(
            drops_text(Some((AckDropsKind::Predicted, 42))),
            " drops: 42 (predicted)"
        );
        assert_eq!(
            drops_text(Some((AckDropsKind::Observed, 7))),
            " drops: 7 (observed)"
        );
        assert_eq!(drops_text(None), "");
    }

    /// identity.
    #[test]
    fn structural_urgent_dedupes_and_heals_by_identity() {
        urgent::clear_by_identity("BRIDGE_PUBLISH_REJECTED", "bridge-broker:structural");
        surface(
            PublishOutcome::Rejected("REJECTED_STRUCTURAL_OP"),
            "gen-A",
            1,
            Some(lqos_dataplane_types::wire::RejectReason::MultiAdd),
            "circuit:c9",
            None,
        );
        surface(
            PublishOutcome::Rejected("REJECTED_STRUCTURAL_OP"),
            "gen-B",
            2,
            Some(lqos_dataplane_types::wire::RejectReason::SpanChanging),
            "site:site-9",
            None,
        );
        let count = || {
            urgent::list()
                .into_iter()
                .filter(|i| i.code == "BRIDGE_PUBLISH_REJECTED")
                .count()
        };
        assert_eq!(count(), 1, "two structural refusals collapse to ONE item");
        let cleared =
            urgent::clear_by_identity("BRIDGE_PUBLISH_REJECTED", "bridge-broker:structural");
        assert_eq!(cleared, 1, "the heal-clear removes it by identity");
        assert_eq!(count(), 0);
    }

    /// The card `docs/bridge_rebuild_alert_clear_card.md`: a REBUILD_REQUIRED
    /// raises ONE stable-identity item (not one per churning generation) and
    /// the apply/heal path clears it by the same identity — the exact mirror of
    /// the structural reject. The oracle: a repeat rebuild makes no 2nd item,
    /// and the heal-clear removes it.
    #[test]
    fn rebuild_required_urgent_dedupes_and_heals_by_identity() {
        urgent::clear_by_identity("BRIDGE_REBUILD_REQUIRED", "bridge-broker:rebuild");
        surface(PublishOutcome::RebuildRequired, "gen-A", 1, None, "", None);
        surface(PublishOutcome::RebuildRequired, "gen-B", 2, None, "", None);
        let count = || {
            urgent::list()
                .into_iter()
                .filter(|i| i.code == "BRIDGE_REBUILD_REQUIRED")
                .count()
        };
        assert_eq!(
            count(),
            1,
            "two rebuild-required outcomes collapse to ONE stable-identity item"
        );
        let cleared =
            urgent::clear_by_identity("BRIDGE_REBUILD_REQUIRED", "bridge-broker:rebuild");
        assert_eq!(cleared, 1, "the apply/heal-clear removes it by identity");
        assert_eq!(count(), 0);
    }
}
