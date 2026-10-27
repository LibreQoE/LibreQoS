//! A3 (`docs/broker_applied_single_source_card.md` §2): the ONE owner of the
//! broker's belief about the bridge's applied world.
//!
//! The bridge's applied-state REPORT is the source of truth; the per-tx
//! ack-fold is an INTERIM projection between reports. This owner is the SINGLE
//! WRITER of the applied belief — `key`/`tx` are never assigned outside it, so
//! `last_applied`/`last_applied_tx` are PROJECTIONS of this type (a "mutation
//! test" against a second writer is therefore structural, not vague).
//!
//! Divergence: when a REPORT disagrees with an INTERIM ack-fold, the report
//! WINS and the divergence is surfaced ([`AppliedState::adopt`]
//! returns `true`; the cadence in `bridge_broker::mod` raises
//! `BRIDGE_APPLIED_DIVERGENCE`).

use lqos_dataplane_types::DataplaneTransaction;
use std::collections::BTreeMap;

/// A2: the PROVENANCE of the applied belief. Surfaced in the convergence report
/// so an operator can tell an ADOPTED base (truth) from a FOLDED one (interim)
/// from a RESET (no belief). The string values are the audit-marker events the
/// instrumentation already emits — the report and the log agree by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseProvenance {
    /// Adopted from the bridge's applied-state report (the TRUTH).
    Report,
    /// Folded from an APPLIED ack (INTERIM, between reports).
    AckFold,
    /// Explicitly reset (re-attach / boot change) — no belief until a report.
    Reset,
}

impl BaseProvenance {
    /// The audit-marker event name (AUDIT GAP 3 vocabulary).
    pub fn as_event(self) -> &'static str {
        match self {
            BaseProvenance::Report => "BASE_ADOPT",
            BaseProvenance::AckFold => "BASE_ACK_FOLD",
            BaseProvenance::Reset => "BASE_RESET_NONE",
        }
    }

    /// The operator-facing label (A2: the report field).
    pub fn as_str(self) -> &'static str {
        self.as_event()
    }
}

/// A2: the CAPABILITY the broker advertises in the convergence report — what the
/// dataplane can apply live.
///
/// ★ R1 (the implementer `7e17f1efc`): these are the broker's LOCALLY-RESOLVED knobs — a
/// PLACEHOLDER for U2, which must carry the **observed** bridge capability from
/// the report. `baseline_apply_armed` is the M-A `Baseline` ARM
/// (`LIBREQOS_DPDK_BASELINE_APPLY`); it is **NOT** the bridge's `placement_mode`
/// (`ctx.placement_bundle.is_some()`, the classify gate) — those DIFFER
/// (jackrabbit: `baseline_apply=false` yet the bundle `Some`). U2 MUST advertise
/// the OBSERVED `placement_mode` from the report; gating on a local arm
/// re-introduces the R2-caution conflation.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BridgeCapability {
    /// The ONE shared circuit bound `K` (the sequence cap / Baseline size gate).
    pub baseline_max_circuits: usize,
    /// The capability/schema version (bump on a contractual change; U2 owns the
    /// contract). N2: named, not a bare literal.
    pub version: u32,
}

/// A2/N2: the capability-contract version (U2 may bump this on a contractual
/// change — e.g. when the observed `placement_mode` is advertised).
pub const CAPABILITY_VERSION: u32 = 1;

impl BridgeCapability {
    /// Resolve the capability from the SAME shared readers the producer and the
    /// bridge use, so the advertised capability cannot drift from the behaviour.
    pub fn resolve() -> Self {
        Self {
            baseline_max_circuits: lqos_dataplane_types::baseline_max_circuits(),
            version: CAPABILITY_VERSION,
        }
    }
}

/// WP3 (`docs/wp2_wp3_fence_convergence_and_adopt_card.md` §2): the bridge's
/// REPORTED applied generation — the ONLY input an adopt may take. TYPED so the
/// broker can never "assume" an adopt (a bare generation string or an ack-fold
/// is not enough): an adopt is an explicit act on a typed report. This is the
/// interface the bridge applied-state report must carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppliedGeneration {
    /// The bridge's reported generation id (visibility / log alignment).
    pub generation: String,
    /// The content key of the reported applied transaction — the reconciler's
    /// baseline identity. Reported by the bridge (or derived by the broker from
    /// the reported transaction bytes); NEVER a locally-folded value.
    pub content_key: String,
    /// `lqos-101`: the DESIRED world's content key reported VERBATIM by the
    /// bridge (the frame key carried with the applied world). `None` = a
    /// pre-fix/boot report; the late-adopt guard then falls back to
    /// `content_key` (old behavior).
    pub desired_world_content_key: Option<String>,
    /// The bridge's authoritative slot order (drains/views), if reported.
    pub slot_order: Option<Vec<String>>,
    /// `lqos-8h8` (a): the per-slot placement owner (`Vec<u16>`, parallel to
    /// `slot_order`), if reported.
    pub owners: Option<Vec<u16>>,
}

/// The ONE owner of the broker's applied belief (A3). The ONLY writer of the
/// applied `key`/`tx`; every consumer reads through the projections below.
#[derive(Debug, Default)]
pub struct AppliedState {
    /// The content key of the last-applied world (the semantic baseline).
    key: Option<String>,
    /// `lqos-101`: the DESIRED world key of the applied generation (the frame
    /// key stored on the in-flight Pending and/or reported by the bridge).
    /// Distinct from `key` (the folded applied key); `None` = pre-fix/boot.
    desired_key: Option<String>,
    /// The last-applied transaction (the structural baseline for the diff).
    tx: Option<DataplaneTransaction>,
    /// Where the belief came from (A2 provenance).
    provenance: Option<BaseProvenance>,
    /// A report OVERRODE a diverging interim ack-fold since the last agreement
    /// (A3 criterion 1/3: the divergence is surfaced; the report wins).
    ///
 /// N4 (the implementer `7e17f1efc`): a `fold_ack` does NOT clear this — the flag
    /// tracks the LATEST reconciliation (cleared by the next adopting report),
    /// so a UI reader must not treat it as sticky-fatal.
    diverged: bool,
    /// `lqos-8h8` (a): the per-circuit PLACEMENT OWNER (worker id) reported by
    /// the bridge, keyed by circuit name (the report's `slot_order` order). Empty
    /// = a pre-owner bridge (the reparent twin is inert). The broker reads this
    /// so it can call the SHARED owner-set predicates without forking placement.
    owners: BTreeMap<String, u16>,
    /// `lqos-74t.118.2`: whether the last adopted report actually CARRIED a
    /// placement-owner table (`slot_order` + `owners`, both present). This
    /// distinguishes a supported zero-circuit world (`owners` empty but
    /// reported) from a pre-owner bridge (`owners` empty because nothing was
    /// reported) — the API must not call the former unavailable.
    owners_reported: bool,
}

/// `lqos-74t.118.2`: decide whether an adopted report actually carried a
/// PLACEMENT-OWNER TABLE worth exposing.
///
/// A pre-owner BOOT report and a true zero-circuit publish both serialize
/// empty `slot_order`/`owners` vectors as CBOR `0x80`, so presence alone is not
/// enough. Accept the empty table ONLY when the applied transaction has no
/// leaves (a supported zero-circuit world); with leaves present, an empty table
/// is the pre-owner/unavailable case and must not render every circuit unowned.
fn owners_reported_from_report(reported: &AppliedGeneration, tx: &DataplaneTransaction) -> bool {
    let both_empty = reported
        .slot_order
        .as_ref()
        .is_some_and(|order| order.is_empty())
        && reported
            .owners
            .as_ref()
            .is_some_and(|owners| owners.is_empty());
    let owners_table = reported
        .slot_order
        .as_ref()
        .is_some_and(|order| !order.is_empty())
        && reported
            .owners
            .as_ref()
            .is_some_and(|owners| !owners.is_empty());
    let tx_has_leaves = tx
        .leaf_ops
        .iter()
        .any(|op| matches!(op, lqos_dataplane_types::DataplaneLeafOp::Upsert { .. }));
    owners_table || (both_empty && !tx_has_leaves)
}

impl AppliedState {
    /// `lqos-8h8` (a): the reported per-circuit placement owner map.
    pub fn owners(&self) -> &BTreeMap<String, u16> {
        &self.owners
    }

    /// `lqos-74t.118.2`: did the last report carry an owner table? `false` =
    /// pre-owner bridge (owner sets/crossings unavailable), `true` with an
    /// empty `owners()` = a supported zero-circuit world.
    pub fn owners_reported(&self) -> bool {
        self.owners_reported
    }

    /// Explicitly DROP the belief (a re-attach / boot change). The next report
    /// re-adopts; until then the broker HOLDS (A1) — it never fabricates.
    pub fn reset(&mut self) {
        self.key = None;
        self.desired_key = None;
        self.tx = None;
        self.provenance = Some(BaseProvenance::Reset);
        self.diverged = false;
        self.owners.clear();
        self.owners_reported = false;
    }

    /// WP3 (`docs/wp2_wp3_fence_convergence_and_adopt_card.md` §2): ADOPT the
    /// bridge's REPORTED applied generation (the TRUTH). This is the ONLY path
    /// that sets a `Report` provenance — the ack-fold is a separate, INTERIM
    /// (optimistic) marker. `reported.content_key` is the bridge's reported
    /// identity (never a locally-folded value). Returns `true` when it OVERRODE
    /// a diverging interim ack-fold (the report WINS).
    pub fn adopt(&mut self, reported: AppliedGeneration, tx: DataplaneTransaction) -> bool {
        let diverged = self.provenance == Some(BaseProvenance::AckFold)
            && self.key.as_deref() != Some(reported.content_key.as_str());
        // Compute before `reported.content_key` is moved into `self.key`.
        let owners_reported = owners_reported_from_report(&reported, &tx);
        self.desired_key = Some(
            reported
                .desired_world_content_key
                .unwrap_or_else(|| reported.content_key.clone()),
        );
        self.key = Some(reported.content_key);
        self.owners_reported = owners_reported;
        self.tx = Some(tx);
        self.owners = match (reported.slot_order.as_ref(), reported.owners.as_ref()) {
            (Some(names), Some(owners)) => {
                names.iter().cloned().zip(owners.iter().copied()).collect()
            }
            _ => BTreeMap::new(),
        };
        self.provenance = Some(BaseProvenance::Report);
        self.diverged = diverged;
        diverged
    }

    /// `lqos-74t.101`: ADOPT a report that proves a LATE application of the
    /// in-flight desired world after `ACK_SILENCE`. This is not a divergence:
    /// the broker deliberately did not settle/terminal the attempt, and the
    /// report is the bridge's fence-committed truth. Keep provenance `Report`
    /// and clear the interim-divergence flag.
    pub fn adopt_late(&mut self, reported: AppliedGeneration, tx: DataplaneTransaction) {
        // Compute before `reported.content_key` is moved into `self.key`.
        let owners_reported = owners_reported_from_report(&reported, &tx);
        self.desired_key = Some(
            reported
                .desired_world_content_key
                .unwrap_or_else(|| reported.content_key.clone()),
        );
        self.key = Some(reported.content_key);
        self.owners_reported = owners_reported;
        self.tx = Some(tx);
        self.owners = match (reported.slot_order.as_ref(), reported.owners.as_ref()) {
            (Some(names), Some(owners)) => {
                names.iter().cloned().zip(owners.iter().copied()).collect()
            }
            _ => BTreeMap::new(),
        };
        self.provenance = Some(BaseProvenance::Report);
        self.diverged = false;
    }

    /// FOLD an APPLIED ack — the INTERIM projection between reports. It keeps
    /// the delta cursor advancing; the NEXT report reconciles (and, if it
    /// disagrees, overrides + surfaces).
    pub fn fold_ack(&mut self, tx: DataplaneTransaction) {
        self.fold_ack_with_desired(tx, None);
    }

    /// `lqos-101`: FOLD an APPLIED ack and STORE the DESIRED world key of the
    /// in-flight publish (the key captured on the Pending at encode time).
    /// `key` remains the folded applied-world key (cache/integrity/divergence
    /// unchanged); `desired_key` is the separate identity the early late-adopt
    /// guard compares. `None` (a pre-fix/sequence publish) keeps the folded
    /// fallback.
    pub fn fold_ack_with_desired(&mut self, tx: DataplaneTransaction, desired_key: Option<String>) {
        let key = super::translate::content_key(&tx);
        // Folded fallback ONLY when the Pending carried no frame key.
        self.desired_key = Some(desired_key.unwrap_or_else(|| key.clone()));
        self.key = Some(key);
        self.tx = Some(tx);
        self.provenance = Some(BaseProvenance::AckFold);
    }

    /// The applied world's content key (the `last_applied` projection).
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    /// `lqos-101`: the DESIRED world key of the applied generation, if known.
    /// The late guard compares this against the report's verbatim desired key.
    pub fn desired_key(&self) -> Option<&str> {
        self.desired_key.as_deref()
    }

    /// The applied transaction (the `last_applied_tx` projection).
    pub fn tx(&self) -> Option<&DataplaneTransaction> {
        self.tx.as_ref()
    }

    /// Whether a base is known (A1: `false` ⇒ HOLD + surface + re-ask).
    pub fn has_base(&self) -> bool {
        self.tx.is_some()
    }

    /// The provenance of the current belief (A2).
    pub fn provenance(&self) -> Option<BaseProvenance> {
        self.provenance
    }

    /// The last adoption OVERRODE a diverging fold (A3).
    pub fn diverged(&self) -> bool {
        self.diverged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny applied tx (one leaf per id) — the content key is derived, not assumed.
    fn tx(generation: &str, circuits: &[&str]) -> DataplaneTransaction {
        use lqos_dataplane_types::{DataplaneLeafAqm, DataplaneRatePair};
        DataplaneTransaction {
            generation: generation.to_string(),
            inner_ops: vec![],
            leaf_ops: circuits
                .iter()
                .map(|id| lqos_dataplane_types::DataplaneLeafOp::Upsert {
                    circuit_id: (*id).to_string(),
                    parent_node_id: "s1".to_string(),
                    download: DataplaneRatePair {
                        commit_kbps: 10_000,
                        ceil_kbps: 20_000,
                    },
                    upload: DataplaneRatePair {
                        commit_kbps: 5_000,
                        ceil_kbps: 10_000,
                    },
                    aqm: DataplaneLeafAqm::None,
                    admin_up: true,
                })
                .collect(),
            mapping_ops: vec![],
            lpm_delta: vec![],
            steering_vlans: Vec::new(),
            schema_version: Some(2),
            apply_mode: Default::default(),
            circuit_capacity: None,
        }
    }

    /// A3 criterion 1: ONE owner is the only writer — reset/adopt/fold are the
    /// sole mutators, and the projections reflect exactly the last write.
    #[test]
    fn single_writer_projections_track_the_owner() {
        let mut applied = AppliedState::default();
        assert!(!applied.has_base(), "a fresh owner has no belief");
        assert_eq!(applied.provenance(), None);

        applied.reset();
        assert_eq!(applied.provenance(), Some(BaseProvenance::Reset));
        assert!(!applied.has_base());
        assert!(applied.key().is_none());

        let world = tx("g1", &["c1"]);
        applied.fold_ack(world.clone());
        assert_eq!(applied.provenance(), Some(BaseProvenance::AckFold));
        assert_eq!(applied.key(), Some(seam_key(&world).as_str()));
        assert_eq!(applied.tx().map(|t| t.generation.as_str()), Some("g1"));
        assert!(!applied.diverged(), "a fold is not itself a divergence");
    }

    /// A3 criterion 3: a report that DISAGREES with a folded ack WINS and is
    /// surfaced (returns `true`); a report that AGREES is not a divergence.
    #[test]
    fn report_overrides_a_diverging_fold_and_is_surfaced() {
        let mut applied = AppliedState::default();
        applied.fold_ack(tx("g-fold", &["c1"]));
        // The report disagrees (a different world).
        let report = tx("g-report", &["c1", "c2"]);
        assert!(
            adopt_of(&mut applied, report.clone()),
            "the report must WIN over a diverging fold and be surfaced"
        );
        assert_eq!(applied.provenance(), Some(BaseProvenance::Report));
        assert_eq!(applied.key(), Some(seam_key(&report).as_str()));
        assert!(applied.diverged(), "the divergence is recorded");
    }

    /// `lqos-101`: fold an APPLIED ack with the DESIRED key carried by the
    /// publish. The report carries that same key (verbatim) while its
    /// transaction is the folded world with a DIFFERENT own key; the next
    /// report therefore AGREES by identity (no false divergence).
    #[test]
    fn fold_ack_with_the_desired_key_agrees_with_the_verbatim_report() {
        let mut applied = AppliedState::default();
        let folded = tx("g1", &["c1"]);
        let desired = tx("g1", &["c1", "c2"]);
        let desired_key = crate::bridge_broker::translate::content_key(&desired);
        assert_ne!(
            desired_key,
            crate::bridge_broker::translate::content_key(&folded)
        );

        let folded_key = crate::bridge_broker::translate::content_key(&folded);
        applied.fold_ack_with_desired(folded.clone(), Some(desired_key.clone()));
        assert_eq!(applied.provenance(), Some(BaseProvenance::AckFold));
        assert_eq!(
            applied.key(),
            Some(folded_key.as_str()),
            "the applied/cache key stays folded"
        );
        assert_eq!(applied.desired_key(), Some(desired_key.as_str()));

        let reported = AppliedGeneration {
            generation: "g1".to_string(),
            content_key: folded_key,
            desired_world_content_key: Some(desired_key.clone()),
            slot_order: None,
            owners: None,
        };
        assert!(
            !applied.adopt(reported, folded),
            "the report's folded key agrees; the desired key is the separate identity"
        );
        assert_eq!(applied.desired_key(), Some(desired_key.as_str()));
        assert!(!applied.diverged());
    }

    /// `lqos-101` boot -> first publish: the boot report adopts the folded key
    /// with no desired field; the first publish's ack-fold stores its frame
    /// desired key; the post-publish report carries the SAME folded key + the
    /// desired field. No false divergence, and the desired identity advances.
    #[test]
    fn boot_to_first_publish_has_no_false_divergence() {
        let mut applied = AppliedState::default();
        let boot = tx("boot", &["c1"]);
        assert!(!adopt_of(&mut applied, boot.clone()));
        assert_eq!(applied.key(), Some(seam_key(&boot).as_str()));
        assert!(!applied.diverged());

        let folded = tx("first", &["c1"]);
        let desired = tx("first", &["c1", "c2"]);
        let desired_key = seam_key(&desired);
        applied.fold_ack_with_desired(folded.clone(), Some(desired_key.clone()));
        assert!(!applied.diverged(), "the boot base is not a divergence");

        let reported = AppliedGeneration {
            generation: "first".to_string(),
            content_key: seam_key(&folded),
            desired_world_content_key: Some(desired_key.clone()),
            slot_order: None,
            owners: None,
        };
        assert!(
            !applied.adopt(reported, folded),
            "the folded keys agree; the desired field is separate"
        );
        assert!(!applied.diverged());
        assert_eq!(applied.desired_key(), Some(desired_key.as_str()));
    }

    /// A report that AGREES with the folded ack is not a divergence (the healthy
    /// interim case).
    #[test]
    fn agreeing_report_is_not_a_divergence() {
        let mut applied = AppliedState::default();
        let world = tx("g1", &["c1"]);
        applied.fold_ack(world.clone());
        assert!(
            !adopt_of(&mut applied, world),
            "an agreeing report is a no-op reconcile"
        );
        assert!(!applied.diverged());
    }

    /// A report adopted over a RESET (no fold) is not a divergence — there was
    /// nothing to disagree with.
    #[test]
    fn report_after_reset_is_not_a_divergence() {
        let mut applied = AppliedState::default();
        applied.reset();
        assert!(!adopt_of(&mut applied, tx("g1", &["c1"])));
        assert!(!applied.diverged());
    }

    /// The provenance labels are the audit-marker vocabulary (A2 contract).
    #[test]
    fn provenance_labels_match_the_audit_events() {
        assert_eq!(BaseProvenance::Report.as_str(), "BASE_ADOPT");
        assert_eq!(BaseProvenance::AckFold.as_str(), "BASE_ACK_FOLD");
        assert_eq!(BaseProvenance::Reset.as_str(), "BASE_RESET_NONE");
    }

    /// `lqos-74t.118.2`: an empty owner table is supported only for a true
    /// zero-leaf world. A pre-owner BOOT report with a restored world also
    /// serializes empty slot_order/owners (CBOR 0x80); it must stay UNAVAILABLE,
    /// not render every restored circuit unowned.
    #[test]
    fn empty_owner_table_is_supported_only_for_a_zero_leaf_world() {
        let boot_world = tx("boot", &["c1"]);
        let boot_key = seam_key(&boot_world);
        let mut boot = AppliedState::default();
        boot.adopt(
            AppliedGeneration {
                generation: "boot".to_string(),
                content_key: boot_key,
                desired_world_content_key: None,
                slot_order: Some(vec![]),
                owners: Some(vec![]),
            },
            boot_world,
        );
        assert!(
            !boot.owners_reported(),
            "pre-owner boot with restored leaves must not claim an owner table"
        );
        assert!(boot.owners().is_empty());

        let zero_world = tx("zero", &[]);
        let zero_key = seam_key(&zero_world);
        let mut zero = AppliedState::default();
        zero.adopt(
            AppliedGeneration {
                generation: "zero".to_string(),
                content_key: zero_key,
                desired_world_content_key: None,
                slot_order: Some(vec![]),
                owners: Some(vec![]),
            },
            zero_world,
        );
        assert!(
            zero.owners_reported(),
            "a true zero-circuit publish is a supported empty owner table"
        );

        let owned_world = tx("owned", &["c1"]);
        let owned_key = seam_key(&owned_world);
        let mut owned = AppliedState::default();
        owned.adopt(
            AppliedGeneration {
                generation: "owned".to_string(),
                content_key: owned_key,
                desired_world_content_key: None,
                slot_order: Some(vec!["c1".to_string()]),
                owners: Some(vec![0]),
            },
            owned_world,
        );
        assert!(owned.owners_reported());
    }

    fn seam_key(tx: &DataplaneTransaction) -> String {
        crate::bridge_broker::translate::content_key(tx)
    }

    /// WP3: adopt a world via the TYPED `AppliedGeneration` (the bridge's
    /// reported applied) — the tests never set the belief directly.
    fn adopt_of(applied: &mut AppliedState, world: DataplaneTransaction) -> bool {
        let reported = AppliedGeneration {
            generation: world.generation.clone(),
            content_key: seam_key(&world),
            desired_world_content_key: None,
            slot_order: None,
            owners: None,
        };
        applied.adopt(reported, world)
    }

    /// WP3 (`docs/wp2_wp3_fence_convergence_and_adopt_card.md` §2): the adopt is
    /// EXPLICIT + TYPED and the base can NOT diverge from the bridge's reported
    /// applied — (a) an ack-fold is INTERIM (optimistic), never authoritative;
    /// (b) a report CONFIRMS it (agreement => not diverged); (c) a report that
    /// disagrees WINS + is surfaced; (d) a reset (bridge restart) drops the base
    /// so the boot report re-adopts.
    #[test]
    fn the_typed_adopt_is_explicit_and_cannot_diverge() {
        // (a) a fold alone is INTERIM (optimistic) — never the `Report` truth.
        let mut applied = AppliedState::default();
        applied.fold_ack(tx("g1", &["c1"]));
        assert_eq!(applied.provenance(), Some(BaseProvenance::AckFold));
        // (b) the SAME world comes back in a report => confirmed, no divergence.
        assert!(!adopt_of(&mut applied, tx("g1", &["c1"])));
        assert_eq!(applied.provenance(), Some(BaseProvenance::Report));
        assert!(!applied.diverged());
        // (c) a report that DISAGREES wins + is surfaced.
        applied.fold_ack(tx("g2", &["c1"]));
        assert!(adopt_of(&mut applied, tx("g2", &["c1", "c2"])));
        assert!(applied.diverged());
        // (d) a restart RESETS the base; the next report re-adopts (no stale base).
        applied.reset();
        assert!(!applied.has_base());
        assert_eq!(applied.provenance(), Some(BaseProvenance::Reset));
        assert!(!adopt_of(&mut applied, tx("g3", &["c1"])));
        assert_eq!(applied.provenance(), Some(BaseProvenance::Report));
    }
}
