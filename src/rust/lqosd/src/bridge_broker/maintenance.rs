//! `[maintenance_window]` — the control-plane publish gate (bead `lqos-8h8`).
//!
//! The broker's `step()` HOLDS a PAUSE-class publish while the site-local window
//! is CLOSED, and re-evaluates each tick; at window-open / force / break-glass it
//! emits. The ENGINE is untouched (card `docs/lqos_maintenance_window_card.md`).
//!
//! ## The predicate is the DERIVED TWIN (§14 pin 2)
//! The pause class is derived from the emitted `DataplaneTransaction` (the
//! `DiffOutcome.delta`) via the SAME SHARED shape predicates the producer uses
//! (`bulk_shape_threshold_for` / `churn_is_full_world_for`), NOT a prose list.
//! Anything unmodelled ⇒ PAUSE-FREE (never a false-defer — §14 pin 1).
//!
//! ## The PRUNED pause set (§14 pin 3)
//! `lpm_delta` (G3) and the same-owner reparent (G1-L2) are PAUSE-FREE and are
//! NOT in the set. The current class set: full-world bulk/reload · `Baseline` ·
//! capacity regrow · kind flip (G2) · the shard-set change (`>4G` add /
//! boundary-crossing re-rate).
//!
//! ## Q10 hole (§14 pin 4) — DOCUMENTED
//! A span-changing reparent is pause-class but the broker has NO ownership input
//! to distinguish it from a pause-free same-owner reparent, so it is NOT
//! classified here (deferring it would false-defer a same-owner move = the §12
//! revert). It stays an ENGINE-side pause. Closing the hole needs the broker to
//! carry the placement/ownership bundle the engine's `content.ownership` has.

use lqos_dataplane_types::{
    ApplyMode, DataplaneInnerOp, DataplaneLeafAqm, DataplaneLeafOp, DataplaneTransaction, LeafRow,
    churn_is_full_world_for, compute_owner_sets, policy3_owner_set_violation,
};
use std::collections::{BTreeMap, HashMap};

/// The engine's shard-tin ceil gate (`shard_tin::shard_eligible`): a circuit at
/// or above this rate is SHARDED. In kbps (the leaf op's rate unit).
pub const SHARD_CEILING_KBPS: u32 = 4_000_000;

/// The DERIVED pause class of an emitted delta. A shape NOT in this set is
/// PAUSE-FREE and is never deferred (§14 pin 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseClass {
    /// A full-world bulk / full reload (the op count exceeds the SHARED
    /// threshold).
    FullWorld,
    /// `apply_mode == Baseline` — the complete-world install.
    Baseline,
    /// A circuit-capacity regrow (the `>4G` add cap).
    CapacityRegrow,
    /// A leaf AQM kind flip (G2). Auto-cake covers the frequent runtime kind
    /// transitions hitless; a deliberate config kind change is rare.
    KindFlip,
    /// A shard-set change: a `>4G` add, or a re-rate crossing the `4G` shard
    /// boundary (either direction).
    ShardSetChange,
    /// A SPAN-CHANGING reparent: a re-parent that changes a node's descendant
    /// OWNER set (the engine's `f3b_reparent_owner_set_belt`). A same-owner
    /// reparent (the owner set is unchanged) is PAUSE-FREE.
    Reparent,
}

impl PauseClass {
    /// A short operator-facing label for the pending banner.
    pub fn label(self) -> &'static str {
        match self {
            PauseClass::FullWorld => "full-world bulk/reload",
            PauseClass::Baseline => "Baseline (full-world install)",
            PauseClass::CapacityRegrow => "capacity regrow",
            PauseClass::KindFlip => "queue-kind flip",
            PauseClass::ShardSetChange => "shard-set change",
            PauseClass::Reparent => "span-changing reparent",
        }
    }
}

/// `lqos-8h8` (a): does `delta` REPARENT a node such that a descendant OWNER set
/// changes — the engine's `f3b_reparent_owner_set_belt` predicate, called on the
/// SAME shared functions (`compute_owner_sets` / `policy3_owner_set_violation`).
/// The owner map comes from the applied-state report (`AppliedState::owners`); an
/// EMPTY map ⇒ inert (a pre-owner bridge).
pub fn reparent_spans_owners(
    applied: &DataplaneTransaction,
    owners: &BTreeMap<String, u16>,
    delta: &DataplaneTransaction,
) -> bool {
    if owners.is_empty() {
        return false;
    }
    let applied_inner_parents: HashMap<String, String> = applied
        .inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert {
                node_id,
                parent_node_id,
                ..
            } => Some((node_id.clone(), parent_node_id.clone())),
            _ => None,
        })
        .collect();
    let applied_leaf_parent: HashMap<&str, &str> = applied
        .leaf_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneLeafOp::Upsert {
                circuit_id,
                parent_node_id,
                ..
            } => Some((circuit_id.as_str(), parent_node_id.as_str())),
            _ => None,
        })
        .collect();
    let reparented_leaves: HashMap<&str, &str> = delta
        .leaf_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneLeafOp::Upsert {
                circuit_id,
                parent_node_id,
                ..
            } if applied_leaf_parent
                .get(circuit_id.as_str())
                .is_some_and(|p| p != parent_node_id) =>
            {
                Some((circuit_id.as_str(), parent_node_id.as_str()))
            }
            DataplaneLeafOp::Move {
                circuit_id,
                new_parent_node_id,
            } => Some((circuit_id.as_str(), new_parent_node_id.as_str())),
            _ => None,
        })
        .collect();
    let mut successor_inner = applied_inner_parents.clone();
    for op in &delta.inner_ops {
        match op {
            DataplaneInnerOp::Upsert {
                node_id,
                parent_node_id,
                ..
            } => {
                successor_inner.insert(node_id.clone(), parent_node_id.clone());
            }
            DataplaneInnerOp::Move {
                node_id,
                new_parent_node_id,
            } => {
                successor_inner.insert(node_id.clone(), new_parent_node_id.clone());
            }
            _ => {}
        }
    }
    let rows: Vec<LeafRow> = applied
        .leaf_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneLeafOp::Upsert {
                circuit_id,
                parent_node_id,
                ..
            } => {
                // Match the engine's fail-closed sentinel: an UNRESOLVED owner is
                // `u16::MAX` (never skipped) so the owner sets are byte-exact
                // against the engine's `compute_owner_sets` input.
                let worker = owners.get(circuit_id).copied().unwrap_or(u16::MAX);
                Some((circuit_id.clone(), parent_node_id.clone(), worker))
            }
            _ => None,
        })
        .collect();
    let owner_sets = compute_owner_sets(&rows, &applied_inner_parents);
    let successor_rows: Vec<LeafRow> = rows
        .iter()
        .map(|(name, parent, owner)| match reparented_leaves.get(name.as_str()) {
            Some(new_parent) => (name.clone(), (*new_parent).to_string(), *owner),
            None => (name.clone(), parent.clone(), *owner),
        })
        .collect();
    let successor_sets = compute_owner_sets(&successor_rows, &successor_inner);
    policy3_owner_set_violation(&owner_sets, &successor_sets).is_some()
}

fn applied_leaf<'a>(
    applied: Option<&'a DataplaneTransaction>,
    circuit_id: &str,
) -> Option<&'a DataplaneLeafOp> {
    applied?.leaf_ops.iter().find(|op| match op {
        DataplaneLeafOp::Upsert { circuit_id: id, .. }
        | DataplaneLeafOp::Remove { circuit_id: id }
        | DataplaneLeafOp::Move { circuit_id: id, .. } => id == circuit_id,
    })
}

fn leaf_ceil_kbps(op: &DataplaneLeafOp) -> u32 {
    match op {
        DataplaneLeafOp::Upsert { download, .. } => download.ceil_kbps,
        DataplaneLeafOp::Remove { .. } | DataplaneLeafOp::Move { .. } => 0,
    }
}

/// A discriminant for the per-leaf AQM kind (the queue-kind identity).
fn leaf_aqm_kind(op: &DataplaneLeafOp) -> Option<u8> {
    match op {
        DataplaneLeafOp::Upsert { aqm, .. } => Some(match aqm {
            DataplaneLeafAqm::None => 0,
            DataplaneLeafAqm::FqCodel => 1,
            DataplaneLeafAqm::Cake { .. } => 2,
        }),
        DataplaneLeafOp::Remove { .. } | DataplaneLeafOp::Move { .. } => None,
    }
}

/// Derives the pause class of `delta` against the `applied` base, using the
/// SHARED shape predicates. `None` ⇒ PAUSE-FREE (emit — never deferred).
pub fn pause_class(
    delta: &DataplaneTransaction,
    applied: Option<&DataplaneTransaction>,
    world: usize,
    override_ops: Option<usize>,
    owners: &BTreeMap<String, u16>,
) -> Option<PauseClass> {
    // Baseline (the complete-world install) is unambiguous.
    if matches!(delta.apply_mode, ApplyMode::Baseline) {
        return Some(PauseClass::Baseline);
    }
    // Capacity regrow (a grow of the circuit capacity).
    if let (Some(cap), Some(base)) = (
        delta.circuit_capacity,
        applied.and_then(|a| a.circuit_capacity),
    ) && cap > base
    {
        return Some(PauseClass::CapacityRegrow);
    }
    // Kind flip + shard-set change: per-leaf against the applied base.
    for op in &delta.leaf_ops {
        let DataplaneLeafOp::Upsert {
            circuit_id,
            download,
            ..
        } = op
        else {
            continue;
        };
        match applied_leaf(applied, circuit_id) {
            Some(base) => {
                if leaf_aqm_kind(op) != leaf_aqm_kind(base) {
                    return Some(PauseClass::KindFlip);
                }
                let base_ceil = leaf_ceil_kbps(base);
                // A re-rate crossing the shard boundary (either direction).
                if (base_ceil > SHARD_CEILING_KBPS) != (download.ceil_kbps > SHARD_CEILING_KBPS) {
                    return Some(PauseClass::ShardSetChange);
                }
            }
            // A new `>4G` circuit is a shard-set change (a fresh sharded member).
            None if download.ceil_kbps > SHARD_CEILING_KBPS => {
                return Some(PauseClass::ShardSetChange);
            }
            None => {}
        }
    }
    // Span-changing reparent (the shared owner-set predicate). A same-owner
    // reparent leaves the owner set unchanged ⇒ PAUSE-FREE (pin 1).
    if let Some(applied) = applied
        && reparent_spans_owners(applied, owners, delta)
    {
        return Some(PauseClass::Reparent);
    }
    // Full-world bulk / full reload: the op count exceeds the SHARED threshold.
    let ops = delta.inner_ops.len() + delta.leaf_ops.len() + delta.mapping_ops.len();
    if churn_is_full_world_for(world, ops, override_ops) {
        return Some(PauseClass::FullWorld);
    }
    None
}

/// Resolves a configured IANA `timezone` to a `chrono_tz::Tz`. `Ok(None)` for
/// the empty (server-local) zone; `Err(())` for an unknown zone name.
pub fn resolve_timezone(timezone: &str) -> Result<Option<chrono_tz::Tz>, ()> {
    let trimmed = timezone.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed.parse::<chrono_tz::Tz>().map(Some).map_err(|_| ())
}

/// The site-local wall-clock minute-of-day (`0..1440`), DST-correct. An empty /
/// `None` zone ⇒ the SERVER-LOCAL zone; a configured IANA zone ⇒ that zone via
/// `chrono-tz`. An UNKNOWN zone name falls back to server-local (the caller
/// warns once at start).
pub fn now_local_minutes(tz: Option<chrono_tz::Tz>) -> u32 {
    use chrono::Timelike;
    match tz {
        Some(tz) => {
            let now = chrono::Utc::now().with_timezone(&tz);
            (now.hour()) * 60 + now.minute()
        }
        None => {
            let now = chrono::Local::now();
            (now.hour()) * 60 + now.minute()
        }
    }
}

/// The held (deferred) change — the pending surface + the force clock.
#[derive(Clone, Debug)]
pub struct DeferredMaintenance {
    /// The class of the HELD (coalesced) delta.
    pub class: PauseClass,
    /// The FIRST time a pause-class publish was held (the force clock).
    pub first_deferred_unix: u64,
    /// The op count of the currently-held coalesced delta (the net change).
    pub held_ops: usize,
    /// Set once the `force_apply_hours` bound (or an override) has fired.
    pub forced: bool,
}

/// The operator OVERRIDE surface (the control file the broker polls each tick):
/// a per-change **apply-now** (keyed on the held content key) + a time-boxed
/// GLOBAL **break-glass** (`until_unix`). Written by the WS handler; audited.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct MaintenanceControl {
    /// Apply the CURRENTLY-HELD change NOW (bypasses the window once). Only one
    /// change is ever held (the coalesced desired), so a boolean suffices.
    #[serde(default)]
    pub apply_now: bool,
    /// The break-glass expiry (unix seconds); while `now < until_unix` ALL held
    /// publishes emit. `None` = no break-glass.
    #[serde(default)]
    pub break_glass_until_unix: Option<u64>,
    /// The operator identity who set the override (the audit trail).
    #[serde(default)]
    pub requested_by: Option<String>,
    /// The request time (unix seconds).
    #[serde(default)]
    pub requested_at_unix: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use lqos_dataplane_types::{ApplyMode, DataplaneInnerOp, DataplaneLeafOp, DataplaneRatePair};

    fn tx(apply_mode: ApplyMode) -> DataplaneTransaction {
        DataplaneTransaction {
            generation: String::new(),
            inner_ops: Vec::new(),
            leaf_ops: Vec::new(),
            mapping_ops: Vec::new(),
            lpm_delta: Vec::new(),
            steering_vlans: Vec::new(),
            schema_version: None,
            apply_mode,
            circuit_capacity: None,
        }
    }

    fn aqm(kind: &str) -> DataplaneLeafAqm {
        match kind {
            "cake" => DataplaneLeafAqm::Cake {
                diffserv4: true,
                rtt_ms: None,
            },
            "fq_codel" => DataplaneLeafAqm::FqCodel,
            _ => DataplaneLeafAqm::None,
        }
    }

    fn leaf(cid: &str, ceil_kbps: u32, kind: &str) -> DataplaneLeafOp {
        DataplaneLeafOp::Upsert {
            circuit_id: cid.to_string(),
            parent_node_id: "site".to_string(),
            download: DataplaneRatePair {
                commit_kbps: ceil_kbps / 4,
                ceil_kbps,
            },
            upload: DataplaneRatePair {
                commit_kbps: ceil_kbps / 4,
                ceil_kbps,
            },
            aqm: aqm(kind),
            admin_up: true,
        }
    }

    #[test]
    fn baseline_is_pause_class() {
        assert_eq!(
            pause_class(&tx(ApplyMode::Baseline), None, 100, None, &BTreeMap::new()),
            Some(PauseClass::Baseline)
        );
    }

    #[test]
    fn small_leaf_churn_is_pause_free() {
        let mut d = tx(ApplyMode::Incremental);
        d.leaf_ops.push(leaf("c-1", 100_000, "cake"));
        assert_eq!(pause_class(&d, None, 50_000, None, &BTreeMap::new()), None);
    }

    #[test]
    fn a_gt_4g_add_is_a_shard_set_change() {
        let mut d = tx(ApplyMode::Incremental);
        d.leaf_ops.push(leaf("c-1", 5_000_000, "cake"));
        assert_eq!(
            pause_class(&d, None, 50_000, None, &BTreeMap::new()),
            Some(PauseClass::ShardSetChange)
        );
    }

    #[test]
    fn a_boundary_crossing_re_rate_is_a_shard_set_change() {
        let mut base = tx(ApplyMode::Incremental);
        base.leaf_ops.push(leaf("c-1", 5_000_000, "cake"));
        let mut d = tx(ApplyMode::Incremental);
        d.leaf_ops.push(leaf("c-1", 2_000_000, "cake"));
        assert_eq!(
            pause_class(&d, Some(&base), 50_000, None, &BTreeMap::new()),
            Some(PauseClass::ShardSetChange)
        );
        // a sub-4G re-rate (both sides below) is pause-free.
        let mut base2 = tx(ApplyMode::Incremental);
        base2.leaf_ops.push(leaf("c-1", 3_000_000, "cake"));
        assert_eq!(pause_class(&d, Some(&base2), 50_000, None, &BTreeMap::new()), None);
    }

    #[test]
    fn a_kind_flip_is_pause_class() {
        let mut base = tx(ApplyMode::Incremental);
        base.leaf_ops.push(leaf("c-1", 100_000, "cake"));
        let mut d = tx(ApplyMode::Incremental);
        d.leaf_ops.push(leaf("c-1", 100_000, "fq_codel"));
        assert_eq!(
            pause_class(&d, Some(&base), 50_000, None, &BTreeMap::new()),
            Some(PauseClass::KindFlip)
        );
    }

    #[test]
    fn an_lpm_delta_only_is_pause_free() {
        let mut d = tx(ApplyMode::Incremental);
        d.lpm_delta
            .push(lqos_dataplane_types::DataplaneMappingOp::Map {
                prefix_len: 32,
                ip: "10.0.0.1".parse().unwrap(),
                circuit_id: "c-1".to_string(),
                device_id: "d-1".to_string(),
            });
        assert_eq!(pause_class(&d, None, 50_000, None, &BTreeMap::new()), None);
    }

    #[test]
    fn a_full_world_is_pause_class() {
        let mut d = tx(ApplyMode::Incremental);
        for i in 0..500 {
            d.leaf_ops.push(leaf(&format!("c-{i}"), 100_000, "cake"));
        }
        assert_eq!(pause_class(&d, None, 500, None, &BTreeMap::new()), Some(PauseClass::FullWorld));
        // the SAME op count on a huge world (below the ×threshold) is pause-free
        // (the shared predicate's world scaling).
        assert_eq!(pause_class(&d, None, 50_000_000, None, &BTreeMap::new()), None);
    }

    fn leaf_parent(cid: &str, parent: &str, ceil_kbps: u32, kind: &str) -> DataplaneLeafOp {
        DataplaneLeafOp::Upsert {
            circuit_id: cid.to_string(),
            parent_node_id: parent.to_string(),
            download: DataplaneRatePair {
                commit_kbps: ceil_kbps / 4,
                ceil_kbps,
            },
            upload: DataplaneRatePair {
                commit_kbps: ceil_kbps / 4,
                ceil_kbps,
            },
            aqm: aqm(kind),
            admin_up: true,
        }
    }

    fn inner(node: &str, parent: &str) -> DataplaneInnerOp {
        DataplaneInnerOp::Upsert {
            node_id: node.to_string(),
            parent_node_id: parent.to_string(),
            download: DataplaneRatePair {
                commit_kbps: 1,
                ceil_kbps: 1,
            },
            upload: DataplaneRatePair {
                commit_kbps: 1,
                ceil_kbps: 1,
            },
        }
    }

    #[test]
    fn span_changing_reparent_defers_same_owner_does_not() {
        let mut applied = tx(ApplyMode::Incremental);
        applied.inner_ops.push(inner("site-a", "root"));
        applied.inner_ops.push(inner("site-b", "root"));
        applied.leaf_ops.push(leaf_parent("c-1", "site-a", 100_000, "cake"));
        applied.leaf_ops.push(leaf_parent("c-2", "site-b", 100_000, "cake"));
        // c-3 keeps site-a non-empty so a same-owner move does NOT empty a node.
        applied.leaf_ops.push(leaf_parent("c-3", "site-a", 100_000, "cake"));
        let mut d = tx(ApplyMode::Incremental);
        d.leaf_ops.push(leaf_parent("c-1", "site-b", 100_000, "cake"));
        // c-1(w0) -> site-b which already holds c-2(w1): site-b gains w0 => a SPAN change.
        let cross = BTreeMap::from([
            ("c-1".to_string(), 0u16),
            ("c-2".to_string(), 1u16),
            ("c-3".to_string(), 0u16),
        ]);
        assert_eq!(
            pause_class(&d, Some(&applied), 50_000, None, &cross),
            Some(PauseClass::Reparent)
        );
        // all on w0 => every owner set is unchanged ({w0}) => PAUSE-FREE (pin 1).
        let same = BTreeMap::from([
            ("c-1".to_string(), 0u16),
            ("c-2".to_string(), 0u16),
            ("c-3".to_string(), 0u16),
        ]);
        assert_eq!(pause_class(&d, Some(&applied), 50_000, None, &same), None);
        // no owner data (a pre-owner bridge) => inert.
        assert_eq!(
            pause_class(&d, Some(&applied), 50_000, None, &BTreeMap::new()),
            None
        );
    }

    #[test]
    fn resolve_timezone_empty_known_unknown() {
        assert_eq!(resolve_timezone(""), Ok(None));
        assert_eq!(resolve_timezone("   "), Ok(None));
        assert!(matches!(resolve_timezone("America/Denver"), Ok(Some(_))));
        assert_eq!(resolve_timezone("Not/AZone"), Err(()));
    }

    #[test]
    fn local_minutes_are_in_range() {
        let tz = resolve_timezone("America/Denver").unwrap();
        assert!(now_local_minutes(tz) < 1440);
        assert!(now_local_minutes(None) < 1440);
    }

    #[test]
    fn a_lone_inner_remove_is_pause_free() {
        let mut d = tx(ApplyMode::Incremental);
        d.inner_ops.push(DataplaneInnerOp::Remove {
            node_id: "site-1".to_string(),
        });
        // (an inner Move — a reparent — is the documented Q10 hole; not deferred)
        assert_eq!(pause_class(&d, None, 50_000, None, &BTreeMap::new()), None);
    }
}
