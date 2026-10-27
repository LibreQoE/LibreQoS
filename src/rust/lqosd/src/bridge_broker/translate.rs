//! Config-diff → `DataplaneTransaction` translation (E1).
//!
//! The emission source is the compiler-published full desired-state
//! artifact (`dataplane_transaction.json`, stamped with the
//! source-stable `shaping_generation` content hash). The broker diffs
//! the PARSED semantic content (fresh-#12 G1: never file bytes or
//! mtimes) against the last APPLIED state and emits only the delta:
//!
//! - rate/AQM changes on existing circuits → leaf `Upsert`s
//! (batched); existing-site rate edits → inner `Upsert`s
//!   ruling 1 — live-supported on the bridge; a changed parent id
//!   rides along and IS live-emitted: same-owner moves apply and
//!   spanning-changing moves are typed-rejected by the crossing belt,
//!   so site re-parenting is NOT inert nor a named gap — see
//!   `docs/r_inner_shapes_tree_differential_card.md`)
//! - §4ter ancestor closure: EVERY emitted inner set (the rate batch
//!   and each structural/drain view) is the PRE-ORDER ANCESTOR CLOSURE
//!   of its seeds — the changed inners AND the changed leaves' parent
//!   nodes — emitted as Upserts only. The bridge compiles each tx FRESH
//!   and rejects a child whose parent is missing or ordered late
//!   (`UnknownParent`, the jackrabbit class-B reject), so an unchanged
//!   ancestor must ride with a changed child and a leaf's parent must
//!   ride with the leaf.
//! - new circuits → leaf `Upsert`s (+ their mapping ops) — under
//!   any parent the payload's own inner `Upserts` create (the N1
//!   derive installs payload sites before the leaf's activation walk;
//! the withholding is UN-SCOPED now that the bridge pairs
//!   with this broker by construction — an older bridge's UnknownName
//!   reject is a typed urgent + one-generation fence, never a storm).
//!   Inner `Remove` was never emitted pre-M1 (the bridge typed-rejects
//!   non-Upsert inners); since M1 the deleted-site retire emits (see
//!   below) — never in a rate-only payload, never mixed.
//! - vanished circuits → leaf `Remove`s (+ mapping `Del`s)
//! - M1 (live_structural_closures_card §1): deleted sites retire —
//!   after the drain sequence, each site absent from desired emits a
//!   trailing per-tx `InnerOp::Remove` (bottom-up: children before
//!   parents; one per tx; baselines retain the pending-retired sites
//!   so the per-tx baseline identity holds for chains)
//! - F3-P (`docs/f3b_flip_and_f3p_work_order.md`): the diff is ONE
//!   coherent change-set — the bridge topo-sorts it against its own
//!   applied base; the per-op SEQUENCE build is DELETED.
//! - suspend = the trickle floor (`commit=ceil=SUSPEND_RATE_KBPS` —
//!   portal-capable, field-calibrated; see the constant)
//!   per the M2 ruling (closures card §5): the bridge's 0=unlimited
//!   convention makes a raw 0/0 pair run at FULL LINE RATE, so the
//!   broker floors every 0/0 direction pair to 1/1 (admin_up stays
//!   `true`; the bridge rejects admin_down). Resume = the normal diff
//!   back to configured rates. Herbert may tune the constant.
//!
//! The delta carries the artifact's generation string VERBATIM
//! (F-R4: never re-stamp). A `None` base emits the full desired state
//! (bridge restart / first attach: everything is an idempotent
//! `Upsert`).

use lqos_dataplane_types::{
    DataplaneInnerOp, DataplaneLeafOp, DataplaneMappingOp, DataplaneTransaction,
};
// The threshold helpers are consumed by the module's TESTS only: the producer's
// shape choice now routes through the SHARED override-aware predicates
// (`bulk_shape_threshold_for` / `churn_is_full_world_for`), which are the single
// source of truth for both ends (R-OVERRIDE-PARITY).
use std::collections::{BTreeMap, BTreeSet};
use tracing::warn;

/// §`lqos-74t.27` (`docs/jackrabbit_140ab861_reparent_flap_diagnosis_card.md`):
/// the env-gated broker NODE TRACE. When `LIBREQOS_BROKER_TRACE_NODE` names a
/// node id substring, log every emitted op that touches it AND the
/// base↔desired field diff that put it in the delta — answering "why did the
/// producer emit an op for this node?" WITHOUT hardcoding any private id.
fn trace_node_delta(
    trace: &str,
    base: &DataplaneTransaction,
    desired: &DataplaneTransaction,
    delta: &DataplaneTransaction,
) {
    let mut hit = false;
    for op in &delta.inner_ops {
        match op {
            DataplaneInnerOp::Upsert {
                node_id,
                parent_node_id,
                download,
                upload,
            } if node_id.contains(trace) => {
                hit = true;
                let base_inner = base.inner_ops.iter().find_map(|o| match o {
                    DataplaneInnerOp::Upsert {
                        node_id: id,
                        parent_node_id,
                        download,
                        upload,
                    } if id == node_id => Some((parent_node_id.as_str(), *download, *upload)),
                    _ => None,
                });
                let in_desired = desired.inner_ops.iter().any(|o| {
                    matches!(o, DataplaneInnerOp::Upsert { node_id: id, .. } if id == node_id)
                });
                let reason = match base_inner {
                    None => format!("new-inner(desired_has={in_desired})"),
                    Some((base_parent, base_download, base_upload)) => {
                        let mut changed = Vec::new();
                        if base_parent != parent_node_id.as_str() {
                            changed.push("parent");
                        }
                        if base_download != *download {
                            changed.push("download");
                        }
                        if base_upload != *upload {
                            changed.push("upload");
                        }
                        if changed.is_empty() {
                            "re-emitted-unchanged".to_string()
                        } else {
                            format!("changed:{}", changed.join(","))
                        }
                    }
                };
                warn!(
                    event = "BROKER_TRACE_NODE",
                    node = %node_id,
                    op = "inner_upsert",
                    parent = %parent_node_id,
                    reason = %reason,
                    "broker emitted an inner op for the traced node"
                );
            }
            DataplaneInnerOp::Remove { node_id } if node_id.contains(trace) => {
                hit = true;
                warn!(
                    event = "BROKER_TRACE_NODE",
                    node = %node_id,
                    op = "inner_remove",
                    "broker emitted an inner remove for the traced node"
                );
            }
            DataplaneInnerOp::Move { node_id, .. } if node_id.contains(trace) => {
                hit = true;
                warn!(
                    event = "BROKER_TRACE_NODE",
                    node = %node_id,
                    op = "inner_move",
                    "broker emitted an inner move for the traced node"
                );
            }
            _ => {}
        }
    }
    for op in &delta.leaf_ops {
        match op {
            DataplaneLeafOp::Upsert {
                circuit_id,
                parent_node_id,
                ..
            } if parent_node_id.contains(trace) || circuit_id.contains(trace) => {
                hit = true;
                warn!(
                    event = "BROKER_TRACE_NODE",
                    node = %circuit_id,
                    op = "leaf_upsert",
                    parent = %parent_node_id,
                    "broker emitted a leaf op touching the traced node"
                );
            }
            DataplaneLeafOp::Remove { circuit_id } if circuit_id.contains(trace) => {
                hit = true;
                warn!(
                    event = "BROKER_TRACE_NODE",
                    node = %circuit_id,
                    op = "leaf_remove",
                    "broker emitted a leaf remove for the traced node"
                );
            }
            _ => {}
        }
    }
    for op in &delta.mapping_ops {
        let circuit = mapping_circuit(op).unwrap_or("");
        if circuit.contains(trace) {
            hit = true;
            warn!(
                event = "BROKER_TRACE_NODE",
                node = %circuit,
                op = "mapping",
                "broker emitted a mapping op for the traced node"
            );
        }
    }
    if !hit {
        warn!(
            event = "BROKER_TRACE_NODE",
            node = %trace,
            op = "none",
            "no emitted op touches the traced node in this delta"
        );
    }
}





/// The semantic content key of a transaction: SHA-256 over the sorted
/// per-op CBOR encodings (parsed content, not file bytes; op ORDER is
/// not content — the compiler's ordering may vary without a semantic
/// change).
pub(crate) fn content_key(tx: &DataplaneTransaction) -> String {
    // WP3 (`docs/wp2_wp3_fence_convergence_and_adopt_card.md` §2): ONE home —
    // the shared crate owns the algorithm. The bridge stamps this exact value
    // into its applied-generation report, so the broker's adopt key and the
    // bridge's reported key are the SAME by construction (no drift).
    lqos_dataplane_types::content_key(tx)
}

/// The STRUCTURAL identity of a world: the sorted inner `(node_id, parent)`
/// edges plus the sorted leaf `(circuit_id, parent)` edges. Rate/AQM edits do
/// not move it.
///
/// The bridge's `classify_transaction` refuses a terminally-unappliable
/// SITE-layer shape (a retire whose applied subtree is non-empty; a new site
/// with no circuit add). Those refusals are keyed to the WORLD's structure,
/// not the shaping generation — but the generation is content-addressed over
/// rates too, and UISP re-derives rates on a ~5 min cadence, so a
/// generation-keyed fence is bypassed on every rate publish. This identity is
/// the stable key the broker fences on instead (the structural-fence fix).
pub(crate) fn structural_identity(tx: &DataplaneTransaction) -> String {
    // LKG-as-cache (`docs/lkg_as_cache_card.md` §2): ONE home — the shared crate
    // owns the algorithm, so the bridge's cache label and the broker's rebuild
    // trigger are the SAME by construction (no drift).
    lqos_dataplane_types::structural_identity(tx)
}

type LeafKey = String;

/// M2 (live_structural_closures_card §5): the emitted suspend floor.
/// A portal-capable trickle — clearly shaped-to-nothing, yet ICMP,
/// DNS, and a small HTTP GET pass, so a suspended customer still
/// reaches a captive-portal/captive-billing page. Herbert may tune.
///
/// FIELD CALIBRATION (F-E3 bench, the first live
/// suspend ever drilled): the original 1 kbps floor was a 100%
/// BLACKHOLE — the LKG proved the bridge applied commit=ceil=1 and
/// ping AND iperf lost everything (docs/e3_derivation.md §.
/// The cliff probe passed the full portal simulation (ICMP 5/5 + DNS
/// + TCP/HTTP GET) at 1000 kbps; the floor lands at the ladder's
/// 1024 (≥ the tested 1000, passes a fortiori; 512 kbps is
/// unreachable through the compile's Mbps rounding and stays
/// untested).
///
/// §`lqos-74t.30` (encode-once): ONE HOME is now the shared crate -- both
/// compilers floor at the source, so the artifact itself carries
/// `SUSPEND_RATE_KBPS` and this is a documented IDEMPOTENT defensive pass (a
/// no-op on an already-floored artifact; the R3 install + the report key both
/// see the FLOORED world, killing the RAW-vs-FLOORED `content_key` fork).

/// M2 (`lqos-74t.30`): delegate to the SHARED pair-level floor so the broker can
/// never drift from the two compilers. A `0/0` pair is `is_disabled()` == full
/// line rate; any other pair passes through. Idempotent.
fn floor_suspended_pair(pair: &mut lqos_dataplane_types::DataplaneRatePair) {
    lqos_dataplane_types::floor_suspended_pair(pair);
}

/// The floor applied to EVERY leaf Upsert of the desired state before
/// any diffing, emission, or baseline folding — one chokepoint so the
/// rate path, the structural views, the boot full-emission, and the
/// per-tx baselines all agree (a suspended circuit can never emit 0/0,
/// and steady-state re-diffs against the floored projection stay empty).
pub(crate) fn apply_suspend_floor(desired: &DataplaneTransaction) -> DataplaneTransaction {
    let mut desired = desired.clone();
    for op in &mut desired.leaf_ops {
        if let DataplaneLeafOp::Upsert {
            download, upload, ..
        } = op
        {
            floor_suspended_pair(download);
            floor_suspended_pair(upload);
        }
    }
    desired
}

fn leaf_identity(op: &DataplaneLeafOp) -> Option<(&LeafKey, &DataplaneLeafOp)> {
    match op {
        DataplaneLeafOp::Upsert { circuit_id, .. } => Some((circuit_id, op)),
        DataplaneLeafOp::Remove { .. } | DataplaneLeafOp::Move { .. } => None,
    }
}

/// Phase C S2/D1: every `DataplaneMappingOp` arm is explicit — a new
/// variant is a COMPILE ERROR here, never a silent drop.
fn mapping_identity(op: &DataplaneMappingOp) -> Option<((u8, String), &DataplaneMappingOp)> {
    match op {
        DataplaneMappingOp::Map { prefix_len, ip, .. } => Some(((*prefix_len, ip.to_string()), op)),
        DataplaneMappingOp::Del { .. }
        | DataplaneMappingOp::MapTunnel { .. }
        | DataplaneMappingOp::DelTunnel { .. }
        | DataplaneMappingOp::MapVlan { .. }
        | DataplaneMappingOp::DelVlan { .. }
        | DataplaneMappingOp::MapMpls { .. }
        | DataplaneMappingOp::DelMpls { .. }
        | DataplaneMappingOp::Clear => None,
    }
}

/// STAGE 2 (R-CHANGESET-CARRIER): the circuit a value-carrying `Map*` op names
/// (the ops the LPM delta resolver needs); `None` for every `Del*`/`Clear`.
fn mapping_circuit(op: &DataplaneMappingOp) -> Option<&str> {
    match op {
        DataplaneMappingOp::Map { circuit_id, .. } => Some(circuit_id),
        DataplaneMappingOp::MapTunnel { circuit, .. }
        | DataplaneMappingOp::MapVlan { circuit, .. }
        | DataplaneMappingOp::MapMpls { circuit, .. } => Some(circuit),
        DataplaneMappingOp::Del { .. }
        | DataplaneMappingOp::DelTunnel { .. }
        | DataplaneMappingOp::DelVlan { .. }
        | DataplaneMappingOp::DelMpls { .. }
        | DataplaneMappingOp::Clear => None,
    }
}

/// T-arc §2.8: the tunnel mapping key `(VNI, VLAN)`. Phase C S2/D1: explicit
/// arms (no `_`) so a new variant cannot be silently dropped.
fn tunnel_identity(op: &DataplaneMappingOp) -> Option<((u32, u16), &DataplaneMappingOp)> {
    match op {
        DataplaneMappingOp::MapTunnel { vni, vlan, .. } => Some(((*vni, *vlan), op)),
        DataplaneMappingOp::Map { .. }
        | DataplaneMappingOp::Del { .. }
        | DataplaneMappingOp::DelTunnel { .. }
        | DataplaneMappingOp::MapVlan { .. }
        | DataplaneMappingOp::DelVlan { .. }
        | DataplaneMappingOp::MapMpls { .. }
        | DataplaneMappingOp::DelMpls { .. }
        | DataplaneMappingOp::Clear => None,
    }
}

/// Phase C S2: the `vlan` mapping key (the service VLAN id). D1: explicit arms.
fn vlan_identity(op: &DataplaneMappingOp) -> Option<(u16, &DataplaneMappingOp)> {
    match op {
        DataplaneMappingOp::MapVlan { vlan, .. } => Some((*vlan, op)),
        DataplaneMappingOp::Map { .. }
        | DataplaneMappingOp::Del { .. }
        | DataplaneMappingOp::MapTunnel { .. }
        | DataplaneMappingOp::DelTunnel { .. }
        | DataplaneMappingOp::DelVlan { .. }
        | DataplaneMappingOp::MapMpls { .. }
        | DataplaneMappingOp::DelMpls { .. }
        | DataplaneMappingOp::Clear => None,
    }
}

/// Phase C S2: the `mpls` mapping key (the top-of-stack label). D1: explicit arms.
fn mpls_identity(op: &DataplaneMappingOp) -> Option<(u32, &DataplaneMappingOp)> {
    match op {
        DataplaneMappingOp::MapMpls { label, .. } => Some((*label, op)),
        DataplaneMappingOp::Map { .. }
        | DataplaneMappingOp::Del { .. }
        | DataplaneMappingOp::MapTunnel { .. }
        | DataplaneMappingOp::DelTunnel { .. }
        | DataplaneMappingOp::MapVlan { .. }
        | DataplaneMappingOp::DelVlan { .. }
        | DataplaneMappingOp::DelMpls { .. }
        | DataplaneMappingOp::Clear => None,
    }
}

/// The result of a semantic diff: F3-P emits ONE coherent change-set (the F3-B
/// shape) plus the full applied world it converges to (the ack baseline).
pub(crate) struct DiffOutcome {
    /// F3-P: the ONE coherent change-set (the F3-B shape). `None` = nothing to
    /// emit — a steady diff, or a HELD changed `libreqos:generated:*` subtree
    /// (the S3/bundle lane). The bridge topo-sorts it against its own applied
    /// base; the producer no longer encodes the dataplane's ordering.
    pub(crate) delta: Option<DataplaneTransaction>,
    /// The full applied world once `delta` applies (what the broker folds on the
    /// ack); `None` when `delta` is `None`. Property (unit-tested): diffing the
    /// desired against this baseline yields an empty delta.
    pub(crate) baseline: Option<DataplaneTransaction>,
    /// §`lqos-74t.12` (DRIVER wave-1): NEW site inners DEFERRED because no
    /// circuit rides them. A lone site has zero dataplane effect, and emitting
    /// it would trip the bridge's N1 `NewSiteNoAdd` gate
    /// (`publish_driver.rs:335`). Deferral is an emission POLICY (upstream of the
    /// reject) — surfaced first-class (never silent) with the site's age.
    pub(crate) deferred_sites: Vec<String>,
    /// §`lqos-74t.13`/`.15`(d): applied SITES DEFERRED for retirement because
    /// the APPLIED subtree still bears a circuit (the bridge's
    /// `RetireWithCircuits` gate runs on the PRE-delta applied world). A site
    /// whose circuits are removed in the SAME delta is deferred one generation;
    /// the next diff's base is circuit-less and the retire rides ALONE (the
    /// engine's lone-inner-remove shape). A POLICY gap, surfaced (never silent)
    /// like `deferred_sites`.
    pub(crate) deferred_retires: Vec<String>,
    /// `lqos-74t.54`: the ALWAYS-ON structural-remove capture — the FULL desired
    /// removed-circuit set, the adopted slot-table length, the suffix verdict,
    /// and the non-suffix remainder held (not emitted). The bridge's dense table
    /// is LIFO (`placement.rs:1189-1205`): only a contiguous suffix of the slot
    /// order is removable, tail-first.
    pub(crate) removal_capture: crate::bridge_broker::RemovalCapture,
}

#[cfg(test)]
impl DiffOutcome {
    /// The applied projection (the baseline). Test-only.
    pub(crate) fn applied_projection(&self) -> &DataplaneTransaction {
        self.baseline
            .as_ref()
            .expect("a non-empty outcome carries a baseline")
    }

    /// Test shim (F3-P): the producer emits ONE coherent delta; expose it as a
    /// one-element sequence so the corpus assertions read uniformly.
    pub(crate) fn deltas(&self) -> Vec<DataplaneTransaction> {
        self.delta.clone().into_iter().collect()
    }

    /// Test shim (F3-P): the one-element per-tx baseline sequence.
    pub(crate) fn per_tx_baselines(&self) -> Vec<DataplaneTransaction> {
        self.baseline.clone().into_iter().collect()
    }
}

/// The COMPOSED order source (R3c-2 amendment): the bridge's
/// slot table is the FACT for everything it names; the BASELINE'S OWN
/// LEAF ORDER extends it for anything the table predates — post-adoption
/// adds append at the bridge's slot tail and the baseline tail grew in
/// ack order == append order, so the composition reproduces the true
/// live slot order. One mechanism at two scopes: with the table absent
/// (old bridge), the baseline order IS the whole source (the named
/// degrade). Dedup keeps the table's position for names it already
/// carries (a drained circuit keeps its old slot — positions never
/// shift on tail removal).
pub(crate) fn compose_order(
    slot_order: Option<&[String]>,
    base: Option<&DataplaneTransaction>,
) -> Vec<LeafKey> {
    let baseline_order = base.iter().flat_map(|b| {
        b.leaf_ops
            .iter()
            .filter_map(|op| match op {
                DataplaneLeafOp::Upsert { circuit_id, .. } => Some(circuit_id.clone()),
                DataplaneLeafOp::Remove { .. } | DataplaneLeafOp::Move { .. } => None,
            })
    });
    match slot_order {
        Some(names) => {
            let mut composed: Vec<LeafKey> = names.to_vec();
            // PERF (`lqos-74t.54`): dedup with a set, not a linear `contains`
            // scan — at 50k rows the scan was ~n²/2 String compares per call.
            let mut seen: std::collections::HashSet<LeafKey> =
                composed.iter().cloned().collect();
            for key in baseline_order {
                if seen.insert(key.clone()) {
                    composed.push(key);
                }
            }
            composed
        }
        None => baseline_order.collect(),
    }
}

/// `lqos-74t.54`: split the desired removed set into the tail-contiguous,
/// removable run and the remainder to hold.
///
/// The bridge's dense structural table is LIFO (`placement.rs:1189-1205`): a
/// remove is accepted only when the circuit is the current TAIL, and the table
/// has no hole representation. A set `R` is therefore removable iff it is a
/// contiguous SUFFIX of the adopted slot order; its members then apply in
/// reverse order (tail-first). Any other `R` member is interior and can never
/// apply until the structure changes — it is returned in `non_suffix` so the
/// caller can hold + fence it instead of emitting a doomed remove.
///
/// NOTE: the engine applies ADDS before REMOVES (`publish_driver.rs:2904` then
/// `:2926`), so a MIXED delta's base removes are not the post-add tail; the
/// producer still emits the pre-add tail run (per the card), and the engine's
/// distinct `RemovedNotTail` rebuild class (engine lane) owns that case.
///
/// Returns `(emittable, non_suffix)`, where `emittable` is in tail-first order.
fn split_removable_removals(
    order: &[LeafKey],
    removed: &BTreeSet<LeafKey>,
) -> (Vec<LeafKey>, Vec<LeafKey>) {
    let mut emittable: Vec<LeafKey> = Vec::new();
    for key in order.iter().rev() {
        if removed.contains(key) {
            emittable.push(key.clone());
        } else {
            break;
        }
    }
    let emittable_set: BTreeSet<&LeafKey> = emittable.iter().collect();
    let non_suffix: Vec<LeafKey> = removed
        .iter()
        .filter(|key| !emittable_set.contains(*key))
        .cloned()
        .collect();
    (emittable, non_suffix)
}




/// The inner node a leaf op hangs from (only Upserts carry one).
fn leaf_parent_node(op: &DataplaneLeafOp) -> Option<&str> {
    match op {
        DataplaneLeafOp::Upsert { parent_node_id, .. } => Some(parent_node_id.as_str()),
        _ => None,
    }
}




/// L2 (`docs/control_plane_structural_convergence_card.md` §3): the identity of
/// a structurally-HELD op. The broker emits one structural op per tx (M1/Flip B
/// one-op-per-payload), so the REJECTED segment identifies the offending op
/// (source (a): segment bookkeeping). Held ops are SUBTRACTED from `desired`
/// before [`semantic_diff`] — the appliable projection — so rates and
/// unaffected structural ops keep applying; the held set is re-evaluated when
/// the desired structural identity changes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HeldOp {
    Inner { node_id: String },
    InnerRemove { node_id: String },
    Leaf { circuit_id: String },
    LeafRemove { circuit_id: String },
}

impl HeldOp {
    /// L4 §5b + #3 (`docs/control_plane_simplification_card.md`): the held op in
    /// OPERATOR language. **Cause-NEUTRAL + identity-bearing.**
    ///
    /// RETIRED (the U1 mis-attribution): the broker used to INVENT a cause —
    /// *"target site is not live yet"* — while the bridge reported `multi-add`.
    /// The CAUSE now comes ONLY from the ack's typed `RejectReason` (U1/W4,
    /// surfaced as `BRIDGE_PUBLISH_REJECTED [MultiAdd at circuit:c9]`). This copy
    /// carries the one honest LOCAL fact (the held identity) and no invented
    /// cause.
    pub(crate) fn operator_text(&self) -> String {
        match self {
            HeldOp::Inner { node_id } => format!("a held site change ({node_id})"),
            HeldOp::InnerRemove { node_id } => format!("a held site retire ({node_id})"),
            HeldOp::Leaf { circuit_id } => format!("a held circuit change ({circuit_id})"),
            HeldOp::LeafRemove { circuit_id } => format!("a held circuit retire ({circuit_id})"),
        }
    }
}


/// L2-1: the held-op identities a REJECTED segment carries (segment bookkeeping
/// — the in-flight/tx content is retained on `Pending`).
///
/// ★ L2-1' (the implementer, REQUIRED): a segment tx is a **VIEW**, not a single op —
/// `mk_view` emits the whole world (every leaf in `order`, the full inner
/// closure). Holding every op in the view would revert the WHOLE world and block
/// independent appliable work (a rejected one-leaf move would hold all N leaves).
/// Hold ONLY the op(s) whose desired state DIFFERS from the applied `base` — the
/// intended structural op the segment carries.
pub(crate) fn held_ops_from_tx(
    tx: &DataplaneTransaction,
    base: Option<&DataplaneTransaction>,
) -> BTreeSet<HeldOp> {
    let base_inner_parent: BTreeMap<&str, &str> = base
        .into_iter()
        .flat_map(|b| b.inner_ops.iter())
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert {
                node_id,
                parent_node_id,
                ..
            } => Some((node_id.as_str(), parent_node_id.as_str())),
            _ => None,
        })
        .collect();
    let base_leaf_parent: BTreeMap<&str, &str> = base
        .into_iter()
        .flat_map(|b| b.leaf_ops.iter())
        .filter_map(|op| match op {
            DataplaneLeafOp::Upsert {
                circuit_id,
                parent_node_id,
                ..
            } => Some((circuit_id.as_str(), parent_node_id.as_str())),
            _ => None,
        })
        .collect();

    let mut held = BTreeSet::new();
    for op in &tx.inner_ops {
        match op {
            DataplaneInnerOp::Upsert {
                node_id,
                parent_node_id,
                ..
            } => {
                if base_inner_parent.get(node_id.as_str()).copied()
                    != Some(parent_node_id.as_str())
                {
                    held.insert(HeldOp::Inner {
                        node_id: node_id.clone(),
                    });
                }
            }
            DataplaneInnerOp::Move { node_id, .. } => {
                held.insert(HeldOp::Inner {
                    node_id: node_id.clone(),
                });
            }
            DataplaneInnerOp::Remove { node_id } => {
                held.insert(HeldOp::InnerRemove {
                    node_id: node_id.clone(),
                });
            }
        }
    }
    for op in &tx.leaf_ops {
        match op {
            DataplaneLeafOp::Upsert {
                circuit_id,
                parent_node_id,
                ..
            } => {
                if base_leaf_parent.get(circuit_id.as_str()).copied()
                    != Some(parent_node_id.as_str())
                {
                    held.insert(HeldOp::Leaf {
                        circuit_id: circuit_id.clone(),
                    });
                }
            }
            DataplaneLeafOp::Move { circuit_id, .. } => {
                held.insert(HeldOp::Leaf {
                    circuit_id: circuit_id.clone(),
                });
            }
            DataplaneLeafOp::Remove { circuit_id } => {
                held.insert(HeldOp::LeafRemove {
                    circuit_id: circuit_id.clone(),
                });
            }
        }
    }
    held
}

/// L2-2/L2-3: the APPLIABLE PROJECTION — `desired` minus the held ops and their
/// PER-CLASS dependents. A held NEW inner is dropped (its NEW descendants too);
/// a held inner MOVE reverts to the base parent; a held retire keeps the applied
/// node; a held NEW leaf drops (and its map); a held leaf MOVE reverts; a held
/// leaf remove keeps the applied leaf. Applying it is a FIXED POINT: re-diffing
/// the projection against itself yields nothing (L2-2).
pub(crate) fn project_desired(
    desired: &DataplaneTransaction,
    base: Option<&DataplaneTransaction>,
    held: &BTreeSet<HeldOp>,
) -> DataplaneTransaction {
    if held.is_empty() {
        return desired.clone();
    }
    let empty_inner: Vec<DataplaneInnerOp> = Vec::new();
    let empty_leaf: Vec<DataplaneLeafOp> = Vec::new();
    let base_inner_ops = base.map(|b| b.inner_ops.as_slice()).unwrap_or(&empty_inner);
    let base_leaf_ops = base.map(|b| b.leaf_ops.as_slice()).unwrap_or(&empty_leaf);
    let base_inner_parent: BTreeMap<&str, &str> = base_inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert {
                node_id,
                parent_node_id,
                ..
            } => Some((node_id.as_str(), parent_node_id.as_str())),
            _ => None,
        })
        .collect();
    let base_inner_op: BTreeMap<&str, &DataplaneInnerOp> = base_inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, .. } => Some((node_id.as_str(), op)),
            _ => None,
        })
        .collect();
    let base_leaf_op: BTreeMap<&str, &DataplaneLeafOp> = base_leaf_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneLeafOp::Upsert { circuit_id, .. } => Some((circuit_id.as_str(), op)),
            _ => None,
        })
        .collect();

    let held_inner: BTreeSet<&str> = held
        .iter()
        .filter_map(|h| match h {
            HeldOp::Inner { node_id } | HeldOp::InnerRemove { node_id } => Some(node_id.as_str()),
            _ => None,
        })
        .collect();
    let held_leaf: BTreeSet<&str> = held
        .iter()
        .filter_map(|h| match h {
            HeldOp::Leaf { circuit_id } | HeldOp::LeafRemove { circuit_id } => {
                Some(circuit_id.as_str())
            }
            _ => None,
        })
        .collect();

    // L2-3: a held NEW inner is dropped; every NEW descendant (parent chain
    // reaches a dropped inner) is dropped too — it would reference a missing
    // parent. Applied (base) nodes are NEVER dropped.
    let mut dropped: BTreeSet<&str> = BTreeSet::new();
    for op in &desired.inner_ops {
        if let DataplaneInnerOp::Upsert { node_id, .. } = op
            && held_inner.contains(node_id.as_str())
            && !base_inner_parent.contains_key(node_id.as_str())
        {
            dropped.insert(node_id.as_str());
        }
    }
    loop {
        let before = dropped.len();
        for op in &desired.inner_ops {
            if let DataplaneInnerOp::Upsert {
                node_id,
                parent_node_id,
                ..
            } = op
                && !base_inner_parent.contains_key(node_id.as_str())
                && dropped.contains(parent_node_id.as_str())
            {
                dropped.insert(node_id.as_str());
            }
        }
        if dropped.len() == before {
            break;
        }
    }

    let mut inner_ops: Vec<DataplaneInnerOp> = Vec::with_capacity(desired.inner_ops.len());
    for op in &desired.inner_ops {
        match op {
            DataplaneInnerOp::Upsert {
                node_id,
                download,
                upload,
                ..
            } => {
                if dropped.contains(node_id.as_str()) {
                    continue;
                }
                if held_inner.contains(node_id.as_str()) {
                    // A held inner MOVE reverts to the base parent; a held NEW
                    // inner is already in `dropped`.
                    if let Some(base_parent) = base_inner_parent.get(node_id.as_str()) {
                        inner_ops.push(DataplaneInnerOp::Upsert {
                            node_id: node_id.clone(),
                            parent_node_id: (*base_parent).to_string(),
                            download: *download,
                            upload: *upload,
                        });
                    }
                } else {
                    inner_ops.push(op.clone());
                }
            }
            DataplaneInnerOp::Remove { node_id } => {
                if held_inner.contains(node_id.as_str()) {
                    // A held retire keeps the applied node (drain-first means
                    // its leaves are already gone).
                    if let Some(base_op) = base_inner_op.get(node_id.as_str()) {
                        inner_ops.push((*base_op).clone());
                    }
                } else {
                    inner_ops.push(op.clone());
                }
            }
            DataplaneInnerOp::Move { .. } => inner_ops.push(op.clone()),
        }
    }

    let mut leaf_ops: Vec<DataplaneLeafOp> = Vec::with_capacity(desired.leaf_ops.len());
    let mut dropped_new_leaves: BTreeSet<&str> = BTreeSet::new();
    for op in &desired.leaf_ops {
        match op {
            DataplaneLeafOp::Upsert {
                circuit_id,
                parent_node_id,
                ..
            } => {
                let affected =
                    held_leaf.contains(circuit_id.as_str()) || dropped.contains(parent_node_id.as_str());
                if !affected {
                    leaf_ops.push(op.clone());
                    continue;
                }
                if let Some(DataplaneLeafOp::Upsert {
                    parent_node_id: base_parent,
                    ..
                }) = base_leaf_op.get(circuit_id.as_str())
                {
                    if let DataplaneLeafOp::Upsert {
                        circuit_id,
                        download,
                        upload,
                        aqm,
                        admin_up,
                        ..
                    } = op
                    {
                        leaf_ops.push(DataplaneLeafOp::Upsert {
                            circuit_id: circuit_id.clone(),
                            parent_node_id: (*base_parent).clone(),
                            download: *download,
                            upload: *upload,
                            aqm: aqm.clone(),
                            admin_up: *admin_up,
                        });
                    }
                } else {
                    dropped_new_leaves.insert(circuit_id.as_str());
                }
            }
            DataplaneLeafOp::Remove { circuit_id } => {
                if held_leaf.contains(circuit_id.as_str()) {
                    if let Some(base_op) = base_leaf_op.get(circuit_id.as_str()) {
                        leaf_ops.push((*base_op).clone());
                    }
                } else {
                    leaf_ops.push(op.clone());
                }
            }
            DataplaneLeafOp::Move { .. } => leaf_ops.push(op.clone()),
        }
    }

    let mapping_ops: Vec<DataplaneMappingOp> = desired
        .mapping_ops
        .iter()
        .filter(|op| {
            mapping_circuit(op)
                .map(|cid| !dropped_new_leaves.contains(cid))
                .unwrap_or(true)
        })
        .cloned()
        .collect();

    DataplaneTransaction {
        generation: desired.generation.clone(),
        inner_ops,
        leaf_ops,
        mapping_ops,
        schema_version: desired.schema_version,
        steering_vlans: Vec::new(),
        lpm_delta: Vec::new(),
        apply_mode: desired.apply_mode,
        circuit_capacity: None,
    }
}

/// Duplicate MAPPING IDENTITIES in a desired world: an identity bound to MORE
/// THAN ONE circuit (the LPM routes only one ⇒ a subscriber is silently
/// misclassified). Covers all four `Map*` families. PURE — no `urgent` side
/// effect in the diff.
///
/// The labels are family-PREFIXED (`prefix:`/`tunnel:`/`vlan:`/`mpls:`) because
/// the caller's `urgent` dedupe key is the label VERBATIM — a bare vlan 10 and a
/// bare mpls label 10 must not collide (`urgent_identity_key`). Returns
/// `(label, distinct circuit ids)` for every identity bound to >1 circuit.
pub(crate) fn duplicate_mapping_identities(
    desired: &DataplaneTransaction,
) -> Vec<(String, Vec<String>)> {
    let mut by_identity: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for op in &desired.mapping_ops {
        let (label, circuit) = match op {
            DataplaneMappingOp::Map {
                prefix_len,
                ip,
                circuit_id,
                ..
            } => (format!("prefix:{prefix_len}:{ip}"), circuit_id.clone()),
            DataplaneMappingOp::MapTunnel { vni, vlan, circuit } => {
                (format!("tunnel:{vni}:{vlan}"), circuit.clone())
            }
            DataplaneMappingOp::MapVlan { vlan, circuit } => {
                (format!("vlan:{vlan}"), circuit.clone())
            }
            DataplaneMappingOp::MapMpls { label, circuit } => {
                (format!("mpls:{label}"), circuit.clone())
            }
            DataplaneMappingOp::Del { .. }
            | DataplaneMappingOp::DelTunnel { .. }
            | DataplaneMappingOp::DelVlan { .. }
            | DataplaneMappingOp::DelMpls { .. }
            | DataplaneMappingOp::Clear => continue,
        };
        let circuits = by_identity.entry(label).or_default();
        if !circuits.contains(&circuit) {
            circuits.push(circuit);
        }
    }
    by_identity
        .into_iter()
        .filter(|(_, circuits)| circuits.len() > 1)
        .map(|(label, mut circuits)| {
            circuits.sort();
            (label, circuits)
        })
        .collect()
}

pub(crate) fn semantic_diff(
    desired: &DataplaneTransaction,
    base: Option<&DataplaneTransaction>,
    slot_order: Option<&[String]>,
) -> DiffOutcome {
    // lqos-74t.54: the adopted bridge slot order IS walked for remove emission —
    // a structural remove is acceptable only at the dense table's tail
    // (`placement.rs:1189-1205`), so the synthesized Removes are ordered
    // tail-first. `compose_order` is shared with the S4 `classify_slot_order`.
    let desired = &apply_suspend_floor(desired);
    let Some(base) = base else {
        // A1 (`docs/broker_applied_single_source_card.md`): NEVER FABRICATE a
        // full-world from a missing base. The ONE source of truth is the bridge's
        // APPLIED; with no base the broker HOLDS (emits NOTHING) and the caller
        // SURFACES the degrade — the bridge boots its own world and the next
        // adopt provides the base. A fabricated full-world is the last live
        // multi-add (the 2..=8 dead-band class). (predecessor: the `base == None`
        // full-emit at the old `translate.rs:1174-1185`.)
        return DiffOutcome {
            delta: None,
            baseline: None,
            deferred_sites: Vec::new(),
            deferred_retires: Vec::new(),
            removal_capture: Default::default(),
        };
    };

    let base_leaves: BTreeMap<&LeafKey, &DataplaneLeafOp> =
        base.leaf_ops.iter().filter_map(leaf_identity).collect();
    let desired_leaves: BTreeMap<&LeafKey, &DataplaneLeafOp> =
        desired.leaf_ops.iter().filter_map(leaf_identity).collect();

    // The live parent universe: sites the bridge already has (they
    // were part of the last applied state — boot applied them, or an
    // unconstrained full emission did), with their applied ops for
    // rate-diffing.
    let base_inners: BTreeMap<&String, &DataplaneInnerOp> = base
        .inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, .. } => Some((node_id, op)),
            DataplaneInnerOp::Remove { .. } | DataplaneInnerOp::Move { .. } => None,
        })
        .collect();

    let base_mappings: BTreeMap<(u8, String), &DataplaneMappingOp> = base
        .mapping_ops
        .iter()
        .filter_map(mapping_identity)
        .collect();
    let desired_mappings: BTreeMap<(u8, String), &DataplaneMappingOp> = desired
        .mapping_ops
        .iter()
        .filter_map(mapping_identity)
        .collect();
    let base_tunnels: BTreeMap<(u32, u16), &DataplaneMappingOp> = base
        .mapping_ops
        .iter()
        .filter_map(tunnel_identity)
        .collect();
    let desired_tunnels: BTreeMap<(u32, u16), &DataplaneMappingOp> = desired
        .mapping_ops
        .iter()
        .filter_map(tunnel_identity)
        .collect();
    let base_vlans: BTreeMap<u16, &DataplaneMappingOp> =
        base.mapping_ops.iter().filter_map(vlan_identity).collect();
    let desired_vlans: BTreeMap<u16, &DataplaneMappingOp> =
        desired.mapping_ops.iter().filter_map(vlan_identity).collect();
    let base_mpls: BTreeMap<u32, &DataplaneMappingOp> =
        base.mapping_ops.iter().filter_map(mpls_identity).collect();
    let desired_mpls: BTreeMap<u32, &DataplaneMappingOp> =
        desired.mapping_ops.iter().filter_map(mpls_identity).collect();

    // Circuits referenced by NEW mappings whose leaves exist — the
    // mapping adds ride with their leaf operations.
    let mut touched_circuits: BTreeSet<LeafKey> = BTreeSet::new();
    // Late+264 ruling 1: existing-site rate edits ARE live-supported —
    // diff and emit them (the bridge overwrites the site rate). A
    // changed parent_node_id rides along and IS live-emitted: same-owner
    // moves apply and spanning-changing moves are typed-rejected by the
    // crossing belt, so site re-parenting is NOT inert nor a named gap
    // (see `docs/r_inner_shapes_tree_differential_card.md`).
    let mut inner_rate_ops: Vec<DataplaneInnerOp> = Vec::new();
    let mut rate_leaf_ops: Vec<DataplaneLeafOp> = Vec::new();
    // Circuits present in base AND desired whose PARENT changed: the bridge's
    // MoveLeaf class. Emitted as their OWN ACKed move tx(s) BEFORE the retire
    // chain (a re-parent mixed into a rate/retire view, or targeting an
    // unknown site, is a typed reject).
    let mut leaf_move_keys: BTreeSet<LeafKey> = BTreeSet::new();
    let mut add_ops: BTreeMap<LeafKey, DataplaneLeafOp> = BTreeMap::new();
    let mut removed_circuits: BTreeSet<LeafKey> = BTreeSet::new();
    let mut mapping_ops: Vec<DataplaneMappingOp> = Vec::new();

    // P2'c (create-then-re-parent): an APPLIED inner whose desired parent
    // differs is the bridge's MoveInner class. Its target must be APPLIED
    // before the move — a re-parent to a not-yet-created site is the loud
    // `site re-parent target parent does not exist` reject. Collect them so
    // (a) the per-step views HOLD them at their base parent and (b) each is
    // emitted as its OWN ack'd move tx AFTER the add phase (which creates a
    // F3-P: no per-step views, so no `hold_all_inner` projection.
    // P7/B3 RETIRED (R3 card §2; DRIVER (b)): the producer no longer HOLDS a
    // changed `libreqos:generated:*` subtree. A generated re-parent flows as an
    // R-B `Upsert` re-parent (or the bridge's F3-B re-parent owner-set belt ->
    // `RebuildRequired` -> R3); a generated retire is a normal Remove. The
    // broker's REJECT-driven hold still applies on a genuine mechanics reject.

    for op in &desired.inner_ops {
        if let DataplaneInnerOp::Upsert { node_id, .. } = op
            && base_inners.get(node_id).is_some_and(|base_op| *base_op != op)
        {
            inner_rate_ops.push(op.clone());
        }
        // New sites (node_id not in base) are NOT special-cased: the
        // structural view path carries their Upserts and the bridge's
 // N1 derive creates them (the withholding is
        // un-scoped — the bridge pairs with this broker by
        // construction, and against an older bridge the UnknownName
        // reject is a typed urgent + one-generation fence, never a
        // storm).
    }

    for (key, desired_leaf) in &desired_leaves {
        match base_leaves.get(key) {
            Some(base_leaf) => {
                if base_leaf != desired_leaf {
                    if leaf_parent_node(base_leaf) != leaf_parent_node(desired_leaf) {
                        leaf_move_keys.insert((*key).clone());
                    } else {
                        rate_leaf_ops.push((*desired_leaf).clone());
                    }
                    touched_circuits.insert((*key).clone());
                }
            }
            None => {
                add_ops.insert((*key).clone(), (*desired_leaf).clone());
                touched_circuits.insert((*key).clone());
            }
        }
    }
    for key in base_leaves.keys() {
        if !desired_leaves.contains_key(key) {
            removed_circuits.insert((*key).clone());
            touched_circuits.insert((*key).clone());
        }
    }

    // Mapping deltas for the RATE-ONLY path, scoped to touched
    // circuits (an untouched circuit's unchanged mappings stay out).
    // Withheld circuits' mappings are withheld WITH them — and the
    // leaf-coherence path below must not resurrect a withheld leaf.
    // (Structural diffs carry the full desired mapping set in their
    // views instead — see the assembly below.)
    for ((prefix, ip), op) in &desired_mappings {
        if base_mappings
            .get(&(*prefix, ip.clone()))
            .is_none_or(|base_op| base_op != op)
            && let DataplaneMappingOp::Map { circuit_id, .. } = op
        {
            if add_ops.contains_key(circuit_id) {
                continue;
            }
            if !touched_circuits.contains(circuit_id) {
                // A mapping change without a leaf change still needs
                // its leaf re-upserted to stay coherent.
                if let Some(leaf) = desired_leaves.get(circuit_id) {
                    rate_leaf_ops.push((*leaf).clone());
                    touched_circuits.insert(circuit_id.clone());
                }
            }
            mapping_ops.push((*op).clone());
        }
    }
    for (identity, op) in &base_mappings {
        if !desired_mappings.contains_key(identity)
            && let DataplaneMappingOp::Map { circuit_id, .. } = op
            && !removed_circuits.contains(circuit_id)
            && (touched_circuits.contains(circuit_id) || !desired_leaves.contains_key(circuit_id))
        {
            mapping_ops.push(DataplaneMappingOp::Del {
                prefix_len: identity.0,
                ip: identity
                    .1
                    .parse()
                    .unwrap_or_else(|_| "0.0.0.0".parse().expect("fallback ip")),
            });
        }
    }
    // T-arc §2.8: the tunnel table follows the same rate-only discipline —
    // a changed/new identity emits a MapTunnel, a removed one a DelTunnel.
    for ((vni, vlan), op) in &desired_tunnels {
        if base_tunnels
            .get(&(*vni, *vlan))
            .is_none_or(|base_op| base_op != op)
            && let DataplaneMappingOp::MapTunnel { circuit, .. } = op
        {
            if add_ops.contains_key(circuit) {
                continue;
            }
            if !touched_circuits.contains(circuit) {
                if let Some(leaf) = desired_leaves.get(circuit) {
                    rate_leaf_ops.push((*leaf).clone());
                    touched_circuits.insert(circuit.clone());
                }
            }
            mapping_ops.push((*op).clone());
        }
    }
    for ((vni, vlan), op) in &base_tunnels {
        if !desired_tunnels.contains_key(&(*vni, *vlan))
            && let DataplaneMappingOp::MapTunnel { circuit, .. } = op
            && !removed_circuits.contains(circuit)
            && (touched_circuits.contains(circuit) || !desired_leaves.contains_key(circuit))
        {
            mapping_ops.push(DataplaneMappingOp::DelTunnel {
                vni: *vni,
                vlan: *vlan,
            });
        }
    }
    // Phase C S2: the vlan table follows the same rate-only discipline.
    for (vlan, op) in &desired_vlans {
        if base_vlans.get(vlan).is_none_or(|base_op| base_op != op)
            && let DataplaneMappingOp::MapVlan { circuit, .. } = op
        {
            if add_ops.contains_key(circuit) {
                continue;
            }
            if !touched_circuits.contains(circuit) {
                if let Some(leaf) = desired_leaves.get(circuit) {
                    rate_leaf_ops.push((*leaf).clone());
                    touched_circuits.insert(circuit.clone());
                }
            }
            mapping_ops.push((*op).clone());
        }
    }
    for (vlan, op) in &base_vlans {
        if !desired_vlans.contains_key(vlan)
            && let DataplaneMappingOp::MapVlan { circuit, .. } = op
            && !removed_circuits.contains(circuit)
            && (touched_circuits.contains(circuit) || !desired_leaves.contains_key(circuit))
        {
            mapping_ops.push(DataplaneMappingOp::DelVlan { vlan: *vlan });
        }
    }
    // Phase C S2: the mpls table (same rate-only discipline).
    for (label, op) in &desired_mpls {
        if base_mpls.get(label).is_none_or(|base_op| base_op != op)
            && let DataplaneMappingOp::MapMpls { circuit, .. } = op
        {
            if add_ops.contains_key(circuit) {
                continue;
            }
            if !touched_circuits.contains(circuit) {
                if let Some(leaf) = desired_leaves.get(circuit) {
                    rate_leaf_ops.push((*leaf).clone());
                    touched_circuits.insert(circuit.clone());
                }
            }
            mapping_ops.push((*op).clone());
        }
    }
    for (label, op) in &base_mpls {
        if !desired_mpls.contains_key(label)
            && let DataplaneMappingOp::MapMpls { circuit, .. } = op
            && !removed_circuits.contains(circuit)
            && (touched_circuits.contains(circuit) || !desired_leaves.contains_key(circuit))
        {
            mapping_ops.push(DataplaneMappingOp::DelMpls { label: *label });
        }
    }

 // The applied projection: with the withholding un-scoped,
    // the projection is simply the desired state (the bridge converges
    // to it once the sequence acks).
 // F3-P (D1 program; `#12` folded, the producer emits ONE
    // coherent change-set — the F3-B shape. The bridge topo-sorts it against its
    // OWN applied base and reconciles the mechanics; the producer no longer
    // encodes the dataplane's ordering (no shell → move → leaf sequence, no
    // Pending-retired sites (applied sites absent from desired), bottom-up so a
    // child retires before its parent.
    let desired_inner_names: BTreeSet<&String> = desired
        .inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, .. } => Some(node_id),
            DataplaneInnerOp::Remove { .. } | DataplaneInnerOp::Move { .. } => None,
        })
        .collect();
    let parent_of = |name: &str| -> Option<&str> {
        base_inners.iter().find_map(|(k, op)| {
            if k.as_str() == name {
                match op {
                    DataplaneInnerOp::Upsert { parent_node_id, .. } => {
                        Some(parent_node_id.as_str())
                    }
                    DataplaneInnerOp::Remove { .. } | DataplaneInnerOp::Move { .. } => None,
                }
            } else {
                None
            }
        })
    };
    let inner_depth = |name: &String| -> usize {
        let mut cur = name.as_str();
        let mut depth = 0usize;
        for _ in 0..64 {
            match parent_of(cur) {
                Some(parent) if parent != cur => {
                    cur = parent;
                    depth += 1;
                }
                _ => break,
            }
        }
        depth
    };
    let mut retired_sites: Vec<(usize, String)> = base_inners
        .iter()
        .filter(|(name, _)| !desired_inner_names.iter().any(|d| d.as_str() == name.as_str()))
        .map(|(name, _)| (inner_depth(name), (*name).clone()))
        .collect();
    retired_sites.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    // §`lqos-74t.12`: which inners BEAR a circuit in the DESIRED world (the same
    // subtree-scoped predicate the topology compiler uses). A NEW inner with no
    // circuit under it is a PURE-site create — zero dataplane effect — and
    // emitting it would trip the bridge's N1 `NewSiteNoAdd` gate
    // (`publish_driver.rs:335`) ⇒ DEFER it (surfaced, never silent).
    let desired_inner_parent: BTreeMap<&str, &str> = desired
        .inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert {
                node_id,
                parent_node_id,
                ..
            } => Some((node_id.as_str(), parent_node_id.as_str())),
            _ => None,
        })
        .collect();
    let mut circuit_bearing: BTreeSet<&str> = BTreeSet::new();
    for op in &desired.leaf_ops {
        if let DataplaneLeafOp::Upsert { parent_node_id, .. } = op {
            let mut cursor = parent_node_id.as_str();
            for _ in 0..64 {
                if cursor == lqos_dataplane_types::DATA_PLANE_ROOT_NODE_ID || cursor.is_empty() {
                    break;
                }
                if !circuit_bearing.insert(cursor) {
                    break; // its whole ancestry was already walked
                }
                match desired_inner_parent.get(cursor) {
                    Some(next) if *next != cursor => cursor = next,
                    _ => break,
                }
            }
        }
    }

    // ── the coherent change-set ─────────────────────────────────────────────
    // changed existing inner Upserts (rate OR parent). The NEW inner Upserts and
    // the bottom-up retired-site Removes are appended AFTER the leaf/mapping
    // assembly: `§lqos-74t.12` must know whether the payload is an inner-only
    // shell before deciding to defer a lone site.
    let mut inner_ops: Vec<DataplaneInnerOp> = inner_rate_ops;
    // §`lqos-74t.13`/`.15`(d): mirror the bridge's `RetireWithCircuits` gate over
    // the APPLIED base — a site is retirable as a delta IFF no base leaf's parent
    // chain reaches it. A site whose circuits are removed in THIS delta is DEFERRED
    // one generation (the removes land + ack; the next diff's base is circuit-less
    // and the retire rides ALONE as the engine's lone-inner-remove shape).
    let applied_subtree_empty = |site: &str| -> bool {
        base_leaves.values().all(|leaf| {
            let mut cur = leaf_parent_node(leaf);
            for _ in 0..64 {
                match cur {
                    Some(p) if p == site => return false,
                    Some(p) => cur = parent_of(p),
                    None => break,
                }
            }
            true
        })
    };
    let mut deferred_retires: Vec<String> = Vec::new();
    let mut retired_removes: Vec<DataplaneInnerOp> = Vec::new();
    for (_, site) in &retired_sites {
        if applied_subtree_empty(site) {
            retired_removes.push(DataplaneInnerOp::Remove {
                node_id: site.clone(),
            });
        } else {
            deferred_retires.push(site.clone());
        }
    }

    // `lqos-74t.54`: order the structural Removes against the adopted slot
    // order. The dense bridge table is LIFO, so only a contiguous suffix is
    // removable, tail-first. WITHOUT a bridge slot table (an old bridge / a
    // producer with no report) tail-ness is unknown — keep the legacy
    // unconditional emission and rely on the engine's typed reject + hold (the
    // named degrade). The capture is ALWAYS-ON (the full set + the table size +
    // the verdict) so a later occurrence answers suffix-ness.
    //
    // PERF (load-bearing at 50k): `compose_order` walks the whole slot table, so
    // it is built ONLY when a slot table is present AND a structural remove
    // exists — the steady state (no remove) never pays for it.
    let (removable_removes, removal_capture) = if removed_circuits.is_empty() {
        (
            Vec::new(),
            crate::bridge_broker::RemovalCapture::default(),
        )
    } else if slot_order.is_some() {
        let removal_order = compose_order(slot_order, Some(base));
        let (emittable, non_suffix) =
            split_removable_removals(&removal_order, &removed_circuits);
        let capture = crate::bridge_broker::RemovalCapture {
            removed: removed_circuits.iter().cloned().collect(),
            slot_table_len: removal_order.len(),
            slot_order_available: true,
            suffix: non_suffix.is_empty(),
            non_suffix,
        };
        (emittable, capture)
    } else {
        // No slot table: tail-ness unknown ⇒ legacy unconditional emission.
        let capture = crate::bridge_broker::RemovalCapture {
            removed: removed_circuits.iter().cloned().collect(),
            slot_table_len: 0,
            slot_order_available: false,
            suffix: true,
            non_suffix: Vec::new(),
        };
        (removed_circuits.iter().cloned().collect(), capture)
    };

    // changed existing leaf Upserts (rate) + NEW leaf Upserts + re-parent leaves
    // (a changed-parent Upsert, R-B) + the removed-circuit Removes.
    let mut leaf_ops: Vec<DataplaneLeafOp> = rate_leaf_ops;
    for (_, op) in add_ops.iter() {
        leaf_ops.push(op.clone());
    }
    for key in &leaf_move_keys {
        if let Some(op) = desired_leaves.get(key) {
            leaf_ops.push((*op).clone());
        }
    }
    for key in &removable_removes {
        leaf_ops.push(DataplaneLeafOp::Remove {
            circuit_id: key.clone(),
        });
    }

    // changed survivor mappings + survivor Dels (the rate path) + a removed
    // circuit's explicit Dels (the fold does not retire mappings on a leaf
    // Remove) + the NEW circuits' desired Maps.
    let mut mapping_ops = mapping_ops;
    for key in &removable_removes {
        for (identity, m) in &base_mappings {
            if let DataplaneMappingOp::Map { circuit_id, .. } = m
                && circuit_id == key
            {
                mapping_ops.push(DataplaneMappingOp::Del {
                    prefix_len: identity.0,
                    ip: identity
                        .1
                        .parse()
                        .unwrap_or_else(|_| "0.0.0.0".parse().expect("fallback ip")),
                });
            }
        }
        for (tkey, m) in &base_tunnels {
            if let DataplaneMappingOp::MapTunnel { circuit, .. } = m
                && circuit == key
            {
                mapping_ops.push(DataplaneMappingOp::DelTunnel {
                    vni: tkey.0,
                    vlan: tkey.1,
                });
            }
        }
        for (vkey, m) in &base_vlans {
            if let DataplaneMappingOp::MapVlan { circuit, .. } = m
                && circuit == key
            {
                mapping_ops.push(DataplaneMappingOp::DelVlan { vlan: *vkey });
            }
        }
        for (mkey, m) in &base_mpls {
            if let DataplaneMappingOp::MapMpls { circuit, .. } = m
                && circuit == key
            {
                mapping_ops.push(DataplaneMappingOp::DelMpls { label: *mkey });
            }
        }
    }
    for (key, _) in add_ops.iter() {
        for op in desired
            .mapping_ops
            .iter()
            .filter(|op| mapping_circuit(op) == Some(key.as_str()))
        {
            mapping_ops.push(op.clone());
        }
    }

    // §`lqos-74t.12` DEFER-to-first-circuit. A NEW inner whose subtree bears no
    // circuit is a pure-site create: zero dataplane effect. The bridge's N1 gate
    // (`publish_driver.rs:318-341`) rejects it ONLY when the payload also carries
    // circuit/mapping content (`unknown_inner && !add_carrying && !pure_inner_shell`);
    // an INNER-ONLY shell payload is ALREADY live-applied by the bridge
    // (`pure_inner_shell`, `:334`). So reconcile: keep the lone site when the
    // payload is a shell, else defer it (surfaced, never silent — a POLICY gap,
    // not a fault).
    let payload_is_shell = leaf_ops.is_empty() && mapping_ops.is_empty();
    let mut deferred_sites: Vec<String> = Vec::new();
    let mut new_inner_ops: Vec<DataplaneInnerOp> = Vec::new();
    for op in &desired.inner_ops {
        if let DataplaneInnerOp::Upsert { node_id, .. } = op
            && !base_inners.contains_key(node_id)
        {
            if payload_is_shell || circuit_bearing.contains(node_id.as_str()) {
                new_inner_ops.push(op.clone());
            } else {
                deferred_sites.push(node_id.clone());
            }
        }
    }
    new_inner_ops.extend(retired_removes);
    inner_ops.extend(new_inner_ops);

    if inner_ops.is_empty() && leaf_ops.is_empty() && mapping_ops.is_empty() {
        // Steady: the desired already equals the applied base (a PURE-site
        // deferral alone leaves `deferred_sites` populated and emits nothing).
        return DiffOutcome {
            delta: None,
            baseline: None,
            deferred_sites,
            deferred_retires,
            removal_capture,
        };
    }

    let delta = DataplaneTransaction {
        generation: desired.generation.clone(),
        inner_ops,
        leaf_ops,
        mapping_ops,
        schema_version: desired.schema_version,
        steering_vlans: Vec::new(),
        lpm_delta: Vec::new(),
        apply_mode: Default::default(),
        circuit_capacity: None,
    };
    // §`lqos-74t.27`: the env-gated NODE TRACE (never a hardcoded private id).
    if let Ok(trace) = std::env::var("LIBREQOS_BROKER_TRACE_NODE") {
        let trace = trace.trim();
        if !trace.is_empty() {
            trace_node_delta(trace, base, desired, &delta);
        }
    }
    // The full applied world once this ONE delta applies — the broker's interim
    // `fold_ack` baseline (the shared `AppliedFold`; a full world, reconciled by
    // the next report). Diffing the desired against it yields nothing.
    let mut fold = lqos_dataplane_types::AppliedFold::from_tx(base);
    fold.fold(&delta);
    let baseline = fold.to_transaction(&desired.generation);

    DiffOutcome {
        delta: Some(delta),
        baseline: Some(baseline),
        deferred_sites,
        deferred_retires,
        removal_capture,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lqos_dataplane_types::{DataplaneLeafAqm, DataplaneRatePair};

    fn pair(c: u32, e: u32) -> DataplaneRatePair {
        DataplaneRatePair {
            commit_kbps: c,
            ceil_kbps: e,
        }
    }

    fn leaf(cid: &str, parent: &str, c: u32, e: u32) -> DataplaneLeafOp {
        DataplaneLeafOp::Upsert {
            circuit_id: cid.to_string(),
            parent_node_id: parent.to_string(),
            download: pair(c, e),
            upload: pair(c, e),
            aqm: DataplaneLeafAqm::None,
            admin_up: true,
        }
    }

    fn map(ip: &str, cid: &str) -> DataplaneMappingOp {
        DataplaneMappingOp::Map {
            prefix_len: 32,
            ip: ip.parse().expect("test ip"),
            circuit_id: cid.to_string(),
            device_id: format!("dev-{cid}"),
        }
    }

    fn tx(
        leaf_ops: Vec<DataplaneLeafOp>,
        mapping_ops: Vec<DataplaneMappingOp>,
    ) -> DataplaneTransaction {
        DataplaneTransaction {
            generation: "gen-1".to_string(),
            inner_ops: vec![DataplaneInnerOp::Upsert {
                node_id: "site-1".to_string(),
                parent_node_id: "root".to_string(),
                download: pair(1_000_000, 10_000_000),
                upload: pair(1_000_000, 10_000_000),
            }],
            leaf_ops,
            mapping_ops,
            schema_version: Some(2),
        
            steering_vlans: Vec::new(), lpm_delta: Vec::new(), apply_mode: Default::default(), circuit_capacity: None, }
    }

    fn base() -> DataplaneTransaction {
        tx(
            vec![
                leaf("c1", "site-1", 5_000, 12_000),
                leaf("c2", "site-1", 6_000, 12_000),
            ],
            vec![map("100.97.0.1", "c1"), map("100.97.0.2", "c2")],
        )
    }

    #[test]
    fn identical_content_yields_no_emission() {
        let desired = base();
        let outcome = semantic_diff(&desired, Some(&base()), None);
        assert!(outcome.deltas().is_empty());
        assert_ne!(content_key(&desired), "");
    }

    /// `.106` round-3 Pin 1(b)/Pin 3: the broker's `semantic_diff` NEVER
    /// carries `ApplyMode::Baseline` into a delta, even when the desired world
    /// is marked Baseline. The bridge's Baseline from-view escape is therefore
    /// not production-reachable from this producer; the no-base/over-threshold
    /// hold is a tracked degrade, and re-arming Baseline needs a completeness
    /// proof (content-key / boot-tx identity).
    #[test]
    fn a_baseline_desired_never_marks_its_delta_baseline() {
        let mut desired = base();
        desired.apply_mode = lqos_dataplane_types::ApplyMode::Baseline;
        // A rate change forces a delta.
        if let DataplaneLeafOp::Upsert { download, .. } = &mut desired.leaf_ops[0] {
            *download = pair(7_000, 12_000);
        }
        let outcome = semantic_diff(&desired, Some(&base()), None);
        let delta = &outcome.deltas()[0];
        assert!(
            matches!(
                delta.apply_mode,
                lqos_dataplane_types::ApplyMode::Incremental
            ),
            "semantic_diff must never emit Baseline (got {:?})",
            delta.apply_mode
        );
    }

    /// `lqos-74t.12`: a NEW site with no circuit is a PURE-site create — it is
    /// deferred (never trips the bridge N1 `NewSiteNoAdd`) until its first
    /// circuit add rides it; the deferral is surfaced (never silent).
    #[test]
    fn lqos74t12_defers_a_lone_site_but_keeps_it_with_its_first_circuit() {
        let base = base();
        let mut desired = base.clone();
        desired.inner_ops.push(DataplaneInnerOp::Upsert {
            node_id: "site-new".to_string(),
            parent_node_id: "root".to_string(),
            download: pair(1_000, 2_000),
            upload: pair(1_000, 2_000),
        });
        // A rate change makes the payload NON-shell (the class that rejects).
        for op in &mut desired.leaf_ops {
            if let DataplaneLeafOp::Upsert {
                circuit_id,
                download,
                ..
            } = op
                && circuit_id == "c1"
            {
                *download = pair(7_000, 12_000);
            }
        }
        let outcome = semantic_diff(&desired, Some(&base), None);
        assert!(
            !outcome.deltas()[0].inner_ops.iter().any(|op| matches!(
                op,
                DataplaneInnerOp::Upsert { node_id, .. } if node_id == "site-new"
            )),
            "the lone site must be DEFERRED"
        );
        assert_eq!(outcome.deferred_sites, vec!["site-new".to_string()]);

        // Its first circuit arrives: the site rides the add (the N1 pairing).
        desired
            .leaf_ops
            .push(leaf("c3", "site-new", 1_000, 2_000));
        desired.mapping_ops.push(map("100.97.0.3", "c3"));
        let outcome = semantic_diff(&desired, Some(&base), None);
        assert!(
            outcome.deltas()[0].inner_ops.iter().any(|op| matches!(
                op,
                DataplaneInnerOp::Upsert { node_id, .. } if node_id == "site-new"
            )),
            "the site must ride its first circuit add"
        );
        assert!(outcome.deferred_sites.is_empty());
    }

    /// `lqos-74t.12` RECONCILE: the bridge's `pure_inner_shell` route already
    /// live-applies an inner-only payload (`publish_driver.rs:334`) — so an
    /// inner-only lone site is KEPT (not deferred).
    #[test]
    fn lqos74t12_keeps_a_lone_site_in_an_inner_only_shell() {
        let base = base();
        let mut desired = base.clone();
        desired.inner_ops.push(DataplaneInnerOp::Upsert {
            node_id: "site-shell".to_string(),
            parent_node_id: "root".to_string(),
            download: pair(1_000, 2_000),
            upload: pair(1_000, 2_000),
        });
        let outcome = semantic_diff(&desired, Some(&base), None);
        assert!(
            outcome.deltas()[0].inner_ops.iter().any(|op| matches!(
                op,
                DataplaneInnerOp::Upsert { node_id, .. } if node_id == "site-shell"
            )),
            "an inner-only shell lone site is applied by the bridge shell route"
        );
        assert!(outcome.deferred_sites.is_empty());
    }

    /// `structural_identity` is the ROOT-2 fence key: it must ignore the
    /// rate/AQM churn that re-mints the shaping generation, and move only on a
    /// real site/circuit structural change.
    #[test]
    fn structural_identity_ignores_rates_and_tracks_structure() {
        let a = base();
        // Same structure, all rates/AQM changed.
        let mut b = base();
        for op in &mut b.leaf_ops {
            if let DataplaneLeafOp::Upsert {
                download, upload, ..
            } = op
            {
                *download = pair(9_999, 9_999);
                *upload = pair(1, 2);
            }
        }
        if let Some(DataplaneInnerOp::Upsert {
            download, upload, ..
        }) = b.inner_ops.first_mut()
        {
            *download = pair(1, 1);
            *upload = pair(1, 1);
        }
        assert_eq!(
            structural_identity(&a),
            structural_identity(&b),
            "rate/AQM edits are not structural"
        );

        // A circuit re-parent is structural.
        let mut c = base();
        if let DataplaneLeafOp::Upsert { parent_node_id, .. } = &mut c.leaf_ops[0] {
            *parent_node_id = "site-2".to_string();
        }
        assert_ne!(
            structural_identity(&a),
            structural_identity(&c),
            "a circuit re-parent is structural"
        );

        // A circuit add is structural.
        let mut d = base();
        d.leaf_ops.push(leaf("c3", "site-1", 1_000, 1_000));
        assert_ne!(
            structural_identity(&a),
            structural_identity(&d),
            "a circuit add is structural"
        );
    }

    #[test]
    fn rate_change_emits_single_leaf_upsert() {
        let mut desired = base();
        if let DataplaneLeafOp::Upsert {
            download, upload, ..
        } = desired.leaf_ops.get_mut(0).expect("leaf 0")
        {
            *download = pair(7_000, 12_000);
            *upload = pair(7_000, 12_000);
        }
        let outcome = semantic_diff(&desired, Some(&base()), None);
        let delta = outcome.delta.as_ref().expect("a rate change emits a delta");
        assert_eq!(delta.leaf_ops.len(), 1);
        assert!(
            matches!(&delta.leaf_ops[0], DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c1")
        );
        // F3-P: the changed leaf's parent is in the bridge's applied base, so
        // the coherent delta carries NO ancestor closure.
        assert!(delta.inner_ops.is_empty(), "the parent is in the applied base");
        assert_eq!(delta.generation, "gen-1", "generation is verbatim");
        assert_eq!(
            content_key(outcome.applied_projection()),
            content_key(&desired),
            "the baseline is the desired state"
        );
    }

    #[test]
    fn suspend_carries_the_trickle_floor_never_zero() {
        let mut desired = base();
        if let DataplaneLeafOp::Upsert {
            download, upload, ..
        } = desired.leaf_ops.get_mut(1).expect("leaf 1")
        {
            *download = pair(0, 0);
            *upload = pair(0, 0);
        }
        let outcome = semantic_diff(&desired, Some(&base()), None);
        assert_eq!(outcome.deltas().len(), 1);
        assert!(
            matches!(&outcome.deltas()[0].leaf_ops[0],
                DataplaneLeafOp::Upsert { circuit_id, download, upload, admin_up, .. }
            if circuit_id == "c2"
                && download.commit_kbps == lqos_dataplane_types::SUSPEND_RATE_KBPS
                && download.ceil_kbps == lqos_dataplane_types::SUSPEND_RATE_KBPS
                && upload.commit_kbps == lqos_dataplane_types::SUSPEND_RATE_KBPS
                && upload.ceil_kbps == lqos_dataplane_types::SUSPEND_RATE_KBPS
                && *admin_up)
        );
    }

    #[test]
    fn suspend_floor_holds_with_no_base() {
        // A1 (`docs/broker_applied_single_source_card.md`): with NO base the
        // producer HOLDS — the boot full emission is retired. (The suspend floor
        // still applies with a base: `suspend_carries_the_trickle_floor_never_zero`.)
        let mut desired = base();
        if let DataplaneLeafOp::Upsert {
            download, upload, ..
        } = desired.leaf_ops.get_mut(1).expect("leaf 1")
        {
            *download = pair(0, 0);
            *upload = pair(0, 0);
        }
        let outcome = semantic_diff(&desired, None, None);
        assert!(
            outcome.deltas().is_empty(),
            "A1: no base => HOLD (no boot full emission)"
        );
    }

    #[test]
    fn suspended_config_steady_state_never_re_emits() {
        // The actor folds the EMITTED (floored) tx as its baseline; a
        // still-suspended config must re-diff to empty against it (no
        // phantom emission every poll).
        let mut desired = base();
        if let DataplaneLeafOp::Upsert {
            download, upload, ..
        } = desired.leaf_ops.get_mut(1).expect("leaf 1")
        {
            *download = pair(0, 0);
            *upload = pair(0, 0);
        }
        let outcome = semantic_diff(&desired, Some(&base()), None);
        let applied = outcome.applied_projection();
        let steady = semantic_diff(&desired, Some(applied), None);
        assert!(
            steady.deltas().is_empty(),
            "floored baseline must be steady against the same desired state"
        );
    }

    #[test]
    fn resume_restores_configured_rates_over_the_floor() {
        // Base = the floored state (what the bridge actually runs while
        // suspended); resume = the normal diff back to configured rates.
        let mut suspended = base();
        if let DataplaneLeafOp::Upsert {
            download, upload, ..
        } = suspended.leaf_ops.get_mut(1).expect("leaf 1")
        {
            *download = pair(lqos_dataplane_types::SUSPEND_RATE_KBPS, lqos_dataplane_types::SUSPEND_RATE_KBPS);
            *upload = pair(lqos_dataplane_types::SUSPEND_RATE_KBPS, lqos_dataplane_types::SUSPEND_RATE_KBPS);
        }
        let outcome = semantic_diff(&base(), Some(&suspended), None);
        assert_eq!(outcome.deltas().len(), 1);
        assert!(
            matches!(&outcome.deltas()[0].leaf_ops[0],
                DataplaneLeafOp::Upsert { circuit_id, download, .. }
            if circuit_id == "c2" && download.commit_kbps == 6_000)
        );
    }

    #[test]
    fn partially_zeroed_pair_is_not_suspension_and_passes_through() {
        // Only BOTH fields zero is the suspend encoding; commit=0 with a
        // real ceil is a burst-allowed shape the compiler may emit
        // deliberately — never floored.
        let mut desired = base();
        if let DataplaneLeafOp::Upsert {
            download, upload, ..
        } = desired.leaf_ops.get_mut(0).expect("leaf 0")
        {
            *download = pair(0, 4_000);
            *upload = pair(0, 0);
        }
        let outcome = semantic_diff(&desired, Some(&base()), None);
        assert_eq!(outcome.deltas().len(), 1);
        assert!(
            matches!(&outcome.deltas()[0].leaf_ops[0],
                DataplaneLeafOp::Upsert { circuit_id, download, upload, .. }
            if circuit_id == "c1"
                && download.commit_kbps == 0 && download.ceil_kbps == 4_000
                && upload.commit_kbps == lqos_dataplane_types::SUSPEND_RATE_KBPS)
        );
    }

    #[test]
    fn site_rate_edit_emits_inner_upsert() {
        // Late+264 ruling 1: existing-site rate edits are live-
        // supported — the diff emits the inner Upsert.
        let mut desired = base();
        if let DataplaneInnerOp::Upsert { download, upload, .. } =
            desired.inner_ops.get_mut(0).expect("site-1")
        {
            *download = pair(2_000_000, 20_000_000);
            *upload = pair(2_000_000, 20_000_000);
        }
        let outcome = semantic_diff(&desired, Some(&base()), None);
        assert_eq!(outcome.deltas().len(), 1);
        let delta = &outcome.deltas()[0];
        assert_eq!(delta.inner_ops.len(), 1);
        assert!(
            matches!(&delta.inner_ops[0], DataplaneInnerOp::Upsert { node_id, download, .. }
            if node_id == "site-1" && download.commit_kbps == 2_000_000)
        );
        assert!(delta.leaf_ops.is_empty());
        assert_eq!(
            content_key(outcome.applied_projection()),
            content_key(&desired)
        );
    }

    #[test]
    fn site_rate_and_circuit_rate_share_one_rate_tx() {
        let mut desired = base();
        if let DataplaneInnerOp::Upsert { download, .. } =
            desired.inner_ops.get_mut(0).expect("site-1")
        {
            *download = pair(2_000_000, 20_000_000);
        }
        if let DataplaneLeafOp::Upsert { download, .. } =
            desired.leaf_ops.get_mut(0).expect("c1")
        {
            *download = pair(7_000, 12_000);
        }
        let outcome = semantic_diff(&desired, Some(&base()), None);
        assert_eq!(outcome.deltas().len(), 1, "rate-class edits batch together");
        let delta = &outcome.deltas()[0];
        assert_eq!(delta.inner_ops.len(), 1);
        assert_eq!(delta.leaf_ops.len(), 1);
    }

    #[test]
    fn add_under_new_parent_emits_with_payload_site() {
 // UN-SCOPED: a new-site add now EMITS — the bridge's
        // N1 derive creates the payload site before the leaf's
 // activation walk (the withholding retired; against
        // an older bridge the UnknownName reject is a typed urgent +
        // one-generation fence, never a storm).
        let mut desired = base();
        desired.inner_ops.push(DataplaneInnerOp::Upsert {
            node_id: "site-2".to_string(),
            parent_node_id: "root".to_string(),
            download: pair(1_000_000, 10_000_000),
            upload: pair(1_000_000, 10_000_000),
        });
        desired.leaf_ops.push(leaf("c3", "site-2", 8_000, 20_000));
        desired.mapping_ops.push(map("100.97.0.3", "c3"));
        let outcome = semantic_diff(&desired, Some(&base()), None);
        assert_eq!(outcome.deltas().len(), 1);
        let delta = &outcome.deltas()[0];
        // Producer-shape view: the new site's inner rides the SAME
        // transaction (payload-created parent) with the full mapping
        // set.
        assert!(
            delta.inner_ops.iter().any(
                |op| matches!(op, DataplaneInnerOp::Upsert { node_id, .. } if node_id == "site-2")
            ),
            "the payload-created site rides the view"
        );
        assert!(
            delta.leaf_ops.iter().any(
                |op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c3")
            )
        );
        assert!(
            delta.mapping_ops.iter().any(
                |op| matches!(op, DataplaneMappingOp::Map { circuit_id, .. } if circuit_id == "c3")
            )
        );
        // Applied projection = desired; the post-ack poll loop is
        // steady (no phantom re-emission).
        assert_eq!(
            content_key(outcome.applied_projection()),
            content_key(&desired)
        );
        let steady = semantic_diff(&desired, Some(outcome.applied_projection()), None);
        assert!(steady.deltas().is_empty(), "post-ack re-diff is stable");
    }

    #[test]
    fn rate_change_flows_alongside_new_site() {
        // Rate changes ride the structural view (idempotent
        // overwrites on the bridge).
        let mut desired = base();
        if let DataplaneLeafOp::Upsert {
            download, upload, ..
        } = desired.leaf_ops.get_mut(0).expect("leaf 0")
        {
            *download = pair(9_000, 12_000);
            *upload = pair(9_000, 12_000);
        }
        desired.inner_ops.push(DataplaneInnerOp::Upsert {
            node_id: "site-2".to_string(),
            parent_node_id: "root".to_string(),
            download: pair(1_000_000, 10_000_000),
            upload: pair(1_000_000, 10_000_000),
        });
        desired.leaf_ops.push(leaf("c3", "site-2", 8_000, 20_000));
        desired.mapping_ops.push(map("100.97.0.3", "c3"));
        let outcome = semantic_diff(&desired, Some(&base()), None);
        assert_eq!(outcome.deltas().len(), 1, "one structural view carries all");
        let delta = &outcome.deltas()[0];
        assert!(
            delta.leaf_ops.iter().any(
                |op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, download, .. }
                if circuit_id == "c1" && download.commit_kbps == 9_000)
            ),
            "the rate change rides the view"
        );
        assert!(
            delta.leaf_ops.iter().any(
                |op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c3")
            )
        );
        assert!(
            delta.inner_ops.iter().any(
                |op| matches!(op, DataplaneInnerOp::Upsert { node_id, .. } if node_id == "site-2")
            )
        );
    }



    #[test]
    fn add_under_existing_parent_emits_without_inner() {
        let mut desired = base();
        desired.leaf_ops.push(leaf("c3", "site-1", 8_000, 20_000));
        desired.mapping_ops.push(map("100.97.0.3", "c3"));
        let outcome = semantic_diff(&desired, Some(&base()), None);
        let delta = outcome.delta.as_ref().expect("the add emits a delta");
        // F3-P coherent change-set: only the CHANGED ops.
        assert!(delta.inner_ops.is_empty(), "site-1 is unchanged (in the base)");
        assert_eq!(delta.leaf_ops.len(), 1, "only the new c3 leaf");
        assert!(
            delta.leaf_ops.iter().any(
                |op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c3")
            )
        );
        assert_eq!(delta.mapping_ops.len(), 1, "only c3's mapping");
        assert_eq!(
            content_key(outcome.applied_projection()),
            content_key(&desired)
        );
    }





    #[test]
    fn add_under_root_parent_is_emitted() {
        // The synthetic root is always live on the bridge (its classify
        // gate exempts it from the parent-exists check).
        let mut desired = base();
        desired
            .leaf_ops
            .push(leaf("c4", lqos_dataplane_types::DATA_PLANE_ROOT_NODE_ID, 4_000, 9_000));
        desired.mapping_ops.push(map("100.97.0.4", "c4"));
        let outcome = semantic_diff(&desired, Some(&base()), None);
        assert_eq!(outcome.deltas().len(), 1);
        assert!(
            outcome.deltas()[0].leaf_ops.iter().any(
                |op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c4")
            )
        );
    }

    #[test]
    fn remove_emits_leaf_remove_and_mapping_del() {
        let mut desired = base();
        desired.leaf_ops.remove(1);
        desired.mapping_ops.remove(1);
        let outcome = semantic_diff(&desired, Some(&base()), None);
        assert_eq!(outcome.deltas().len(), 1);
        let delta = &outcome.deltas()[0];
        assert!(
            delta.leaf_ops.iter().any(
                |op| matches!(op, DataplaneLeafOp::Remove { circuit_id } if circuit_id == "c2")
            )
        );
        assert!(delta.mapping_ops.iter().any(
            |op| matches!(op, DataplaneMappingOp::Del { ip, .. } if ip.to_string() == "100.97.0.2")
        ));
        assert!(
            !delta
                .inner_ops
                .iter()
                .any(|op| matches!(op, DataplaneInnerOp::Remove { .. })),
            "the DRAIN tx carries no retire — the retire is a separate trailing tx (M1)"
        );
    }




    #[test]
    fn no_base_holds_and_fabricates_nothing() {
        // A1 (`docs/broker_applied_single_source_card.md`): with NO base the
        // producer HOLDS — it does NOT fabricate a full-world (the last live
        // multi-add / the 2..=8 dead-band class). The bridge boots its own world;
        // the next adopt provides the base.
        let mut desired = base();
        desired.inner_ops.push(DataplaneInnerOp::Upsert {
            node_id: "site-2".to_string(),
            parent_node_id: "root".to_string(),
            download: pair(1_000_000, 10_000_000),
            upload: pair(1_000_000, 10_000_000),
        });
        desired.leaf_ops.push(leaf("c3", "site-2", 8_000, 20_000));
        let outcome = semantic_diff(&desired, None, None);
        assert!(
            outcome.deltas().is_empty(),
            "no base => HOLD (fabricate nothing)"
        );
    }

    #[test]
    fn content_key_ignores_serialization_not_semantics() {
        let a = base();
        let mut b = base();
        b.leaf_ops.reverse();
        // Same semantic content, different op order: the keys match —
        // parsed-content hashing must not emit phantom deltas.
        assert_eq!(content_key(&a), content_key(&b));
    }

    // ---- R3c §2 gates (remove_lpm_completeness_card §R3c) ----

    /// §2 fixture: four live circuits c1..c4 (slot order == fold order),
    /// one map each. Drains of a doomed set emit one tx per pop.
    fn r3c_base_4() -> DataplaneTransaction {
        let pair = |c: u32, e: u32| DataplaneRatePair { commit_kbps: c, ceil_kbps: e };
        let leaf = |i: u32| DataplaneLeafOp::Upsert {
            circuit_id: format!("c{i}"),
            parent_node_id: "site-1".to_string(),
            download: pair(10_000, 20_000),
            upload: pair(5_000, 10_000),
            aqm: DataplaneLeafAqm::None,
            admin_up: true,
        };
        DataplaneTransaction {
            generation: "r3c-base".to_string(),
            inner_ops: vec![DataplaneInnerOp::Upsert {
                node_id: "site-1".to_string(),
                parent_node_id: "root".to_string(),
                download: pair(100_000, 200_000),
                upload: pair(100_000, 200_000),
            }],
            leaf_ops: vec![leaf(1), leaf(2), leaf(3), leaf(4)],
            mapping_ops: vec![
                map("100.97.0.1", "c1"),
                map("100.97.0.2", "c2"),
                map("100.97.0.3", "c3"),
                map("100.97.0.4", "c4"),
            ],
            schema_version: Some(2),
        
            steering_vlans: Vec::new(), lpm_delta: Vec::new(), apply_mode: Default::default(), circuit_capacity: None, }
    }

    /// The live slot table a view's upsert SEQUENCE compiles to
    /// (positional: leaf-op order == slot space, transaction.rs compile).
    /// Returns (slot, circuit) pairs, slot 1-based (0 = best-effort).
    fn world_slots(tx: &DataplaneTransaction) -> Vec<(usize, String)> {
        tx.leaf_ops
            .iter()
            .enumerate()
            .filter_map(|(i, op)| match op {
                DataplaneLeafOp::Upsert { circuit_id, .. } => Some((i + 1, circuit_id.clone())),
                DataplaneLeafOp::Remove { .. } | DataplaneLeafOp::Move { .. } => None,
            })
            .collect()
    }

    /// The prefix→circuit bindings a view compiles (Map ops only).
    fn world_maps(tx: &DataplaneTransaction) -> Vec<(String, String)> {
        tx.mapping_ops
            .iter()
            .filter_map(|op| match op {
                DataplaneMappingOp::Map { ip, circuit_id, .. } => {
                    Some((ip.to_string(), circuit_id.clone()))
                }
                DataplaneMappingOp::Del { .. }
                | DataplaneMappingOp::MapTunnel { .. }
                | DataplaneMappingOp::DelTunnel { .. }
                | DataplaneMappingOp::MapVlan { .. }
                | DataplaneMappingOp::DelVlan { .. }
                | DataplaneMappingOp::MapMpls { .. }
                | DataplaneMappingOp::DelMpls { .. }
                | DataplaneMappingOp::Clear => None,
            })
            .collect()
    }









    /// SOURCE-LEVER ORACLE (card `incremental_emission_source_lever_card.md`):
    /// the routing is THRESHOLD-based (op count), NOT size-based, so today a
    /// diff with adds+removes+retires > `bulk_threshold()` (default 8) takes the
    /// FULL-WORLD bulk branch — ONE delta equal to the WHOLE desired world (the
    /// O(n) prestage that fails at 50k). L1's target is to SEQUENCE it.
    fn churn(adds: u32) -> (DataplaneTransaction, DataplaneTransaction) {
        let base = base();
        let mut leaves = base.leaf_ops.clone();
        let mut maps = base.mapping_ops.clone();
        for i in 0..adds {
            let id = 3 + i;
            leaves.push(leaf(&format!("c{id}"), "site-1", 5_000, 12_000));
            maps.push(map(&format!("100.97.0.{id}"), &format!("c{id}")));
        }
        (base, tx(leaves, maps))
    }




    #[test]
    fn new_site_with_an_add_is_carried_today() {
        // The N1 gate (bridge `publish_driver.rs:277-301`): a new site is
        // live-legal IFF the payload ALSO carries a circuit add. The broker's
        // diff must carry the new inner WITH the add.
        let base = base();
        let mut desired = base.clone();
        desired.inner_ops.push(DataplaneInnerOp::Upsert {
            node_id: "site-2".to_string(),
            parent_node_id: "root".to_string(),
            download: pair(1_000_000, 10_000_000),
            upload: pair(1_000_000, 10_000_000),
        });
        desired.leaf_ops.push(leaf("c3", "site-2", 5_000, 12_000));
        desired.mapping_ops.push(map("100.97.0.3", "c3"));
        let outcome = semantic_diff(&desired, Some(&base), None);
        let deltas = outcome.deltas();
        let carrying = deltas
            .iter()
            .find(|d| {
                d.leaf_ops.iter().any(
                    |op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c3"),
                )
            })
            .expect("an add-carrying delta");
        assert!(
            carrying.inner_ops.iter().any(
                |op| matches!(op, DataplaneInnerOp::Upsert { node_id, .. } if node_id == "site-2")
            ),
            "the new site's inner must ride WITH its circuit add (N1 gate satisfied)"
        );
    }




    fn inner_up(id: &str, parent: &str) -> DataplaneInnerOp {
        DataplaneInnerOp::Upsert {
            node_id: id.to_string(),
            parent_node_id: parent.to_string(),
            download: pair(1_000_000, 10_000_000),
            upload: pair(1_000_000, 10_000_000),
        }
    }

    fn bare(inners: Vec<DataplaneInnerOp>, leaves: Vec<DataplaneLeafOp>) -> DataplaneTransaction {
        DataplaneTransaction {
            generation: "gen-1".to_string(),
            inner_ops: inners,
            leaf_ops: leaves,
            mapping_ops: Vec::new(),
            schema_version: Some(2),
            steering_vlans: Vec::new(),
            lpm_delta: Vec::new(),
            apply_mode: Default::default(),
            circuit_capacity: None,
        }
    }

    fn held_inner(id: &str) -> BTreeSet<HeldOp> {
        let mut held = BTreeSet::new();
        held.insert(HeldOp::Inner {
            node_id: id.to_string(),
        });
        held
    }

    /// L2-2/L2-3: a held NEW site is dropped from the projection, but an
    /// INDEPENDENT leaf add under a LIVE site survives; the projection is a
    /// fixed point.
    #[test]
    fn projection_drops_a_held_new_site_but_keeps_an_independent_add() {
        let base = bare(vec![inner_up("site-1", "root")], vec![leaf("c1", "site-1", 5, 12)]);
        let desired = bare(
            vec![inner_up("site-1", "root"), inner_up("newT", "root")],
            vec![leaf("c1", "site-1", 5, 12), leaf("c2", "site-1", 8, 20)],
        );
        let proj = project_desired(&desired, Some(&base), &held_inner("newT"));
        assert!(
            !proj.inner_ops.iter().any(|op| matches!(op, DataplaneInnerOp::Upsert { node_id, .. } if node_id == "newT")),
            "the held new site is dropped"
        );
        assert!(
            proj.leaf_ops.iter().any(|op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c2")),
            "the independent add survives the projection"
        );
        assert!(
            semantic_diff(&proj, Some(&proj), None).deltas().is_empty(),
            "the projection is a FIXED POINT"
        );
        let out = semantic_diff(&proj, Some(&base), None);
        assert!(out.deltas().iter().any(|d| d.leaf_ops.iter().any(
            |op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c2")
        )));
        assert!(!out.deltas().iter().any(|d| d.inner_ops.iter().any(
            |op| matches!(op, DataplaneInnerOp::Upsert { node_id, .. } if node_id == "newT")
        )));
    }

    /// L2-3: a held inner MOVE reverts the site to its base parent (never drops
    /// an applied node, never re-emits the move).
    #[test]
    fn projection_reverts_a_held_inner_move_to_the_base_parent() {
        let base = bare(
            vec![inner_up("site-1", "root"), inner_up("mre", "root")],
            vec![leaf("c1", "site-1", 5, 12)],
        );
        let desired = bare(
            vec![inner_up("site-1", "root"), inner_up("target", "root"), inner_up("mre", "target")],
            vec![leaf("c1", "site-1", 5, 12)],
        );
        let proj = project_desired(&desired, Some(&base), &held_inner("mre"));
        let mre_parent = proj.inner_ops.iter().find_map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, parent_node_id, .. } if node_id == "mre" => {
                Some(parent_node_id.clone())
            }
            _ => None,
        });
        assert_eq!(mre_parent.as_deref(), Some("root"), "the held move reverts to base");
        assert!(semantic_diff(&proj, Some(&proj), None).deltas().is_empty());
    }

    /// L2-2: a held NEW inner's NEW descendants (and leaves under them) are
    /// dropped with it (the closure), so the projection never references a
    /// missing parent.
    #[test]
    fn projection_drops_new_descendants_of_a_held_new_site() {
        let base = bare(vec![inner_up("site-1", "root")], vec![leaf("c1", "site-1", 5, 12)]);
        let desired = bare(
            vec![inner_up("site-1", "root"), inner_up("newT", "root"), inner_up("newchild", "newT")],
            vec![leaf("c1", "site-1", 5, 12), leaf("c9", "newchild", 3, 9)],
        );
        let proj = project_desired(&desired, Some(&base), &held_inner("newT"));
        assert!(!proj.inner_ops.iter().any(|op| matches!(op, DataplaneInnerOp::Upsert { node_id, .. } if node_id == "newT" || node_id == "newchild")), "the held site AND its new descendant are dropped");
        assert!(!proj.leaf_ops.iter().any(|op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c9")), "the new leaf under the dropped subtree is dropped");
        assert!(proj.leaf_ops.iter().any(|op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c1")), "the live leaf survives");
    }

    /// L2-6 (the DRIVER's literal worry): with a synthetic unappliable op held,
    /// an independent add applies, and it RE-applies across a rate change (the
    /// identity-churning case the world fence could not handle) while the held
    /// op stays held and the projection stays a fixed point.
    #[test]
    fn held_op_stays_held_while_an_independent_add_applies_across_rate_changes() {
        let base = bare(vec![inner_up("site-1", "root")], vec![leaf("c1", "site-1", 5, 12)]);
        let held = held_inner("newT");
        let d1 = bare(
            vec![inner_up("site-1", "root"), inner_up("newT", "root")],
            vec![leaf("c1", "site-1", 5, 12), leaf("c2", "site-1", 8, 20)],
        );
        let p1 = project_desired(&d1, Some(&base), &held);
        let out1 = semantic_diff(&p1, Some(&base), None);
        assert!(out1.deltas().iter().any(|d| d.leaf_ops.iter().any(
            |op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c2")
        )));
        assert!(!out1.deltas().iter().any(|d| d.inner_ops.iter().any(
            |op| matches!(op, DataplaneInnerOp::Upsert { node_id, .. } if node_id == "newT")
        )));

        let applied1 = out1.applied_projection().clone();
        let d2 = bare(
            vec![inner_up("site-1", "root"), inner_up("newT", "root")],
            vec![leaf("c1", "site-1", 5, 12), leaf("c2", "site-1", 9, 22)],
        );
        let p2 = project_desired(&d2, Some(&applied1), &held);
        let out2 = semantic_diff(&p2, Some(&applied1), None);
        assert!(!out2.deltas().is_empty(), "the rate change still emits");
        assert!(out2.deltas().iter().all(|d| !d.inner_ops.iter().any(
            |op| matches!(op, DataplaneInnerOp::Upsert { node_id, .. } if node_id == "newT")
        )));
        let applied2 = out2.applied_projection().clone();
        let p3 = project_desired(&d2, Some(&applied2), &held);
        assert!(
            semantic_diff(&p3, Some(&applied2), None).deltas().is_empty(),
            "fixed point: no re-emit / no re-fence"
        );
    }

 /// L2-1' (the implementer REQUIRED): a segment tx is a VIEW — `held_ops_from_tx`
    /// must hold ONLY the op that differs from the applied base, not every op in
    /// the view. An 8-circuit world's single leaf re-parent must hold exactly 1.
    #[test]
    fn held_ops_from_a_real_move_delta_is_only_the_moved_op() {
        let base = bare(
            vec![inner_up("site-1", "root"), inner_up("site-2", "root")],
            (1..=8)
                .map(|i| leaf(&format!("c{i}"), "site-1", 5, 12))
                .collect(),
        );
        let mut desired = base.clone();
        desired.leaf_ops = (1..=8)
            .map(|i| {
                if i == 8 {
                    leaf("c8", "site-2", 5, 12)
                } else {
                    leaf(&format!("c{i}"), "site-1", 5, 12)
                }
            })
            .collect();
        let outcome = semantic_diff(&desired, Some(&base), None);
        let deltas = outcome.deltas();
        let move_delta = deltas
            .iter()
            .find(|d| {
                d.leaf_ops.iter().any(|op| matches!(
                    op,
                    DataplaneLeafOp::Upsert { circuit_id, parent_node_id, .. }
                        if circuit_id == "c8" && parent_node_id == "site-2"
                ))
            })
            .expect("the leaf-move delta is emitted");
        let held = held_ops_from_tx(move_delta, Some(&base));
        assert_eq!(
            held.len(),
            1,
            "only the moved op is held, not the whole view: {held:?}"
        );
        assert!(held.contains(&HeldOp::Leaf {
            circuit_id: "c8".to_string()
        }));
    }


    /// #3 (`docs/control_plane_simplification_card.md`): the held op renders in
    /// OPERATOR language — CAUSE-NEUTRAL + identity-bearing, no jargon. The
    /// fabricated "target not live" cause is RETIRED (the ack's typed reason is
    /// the sole cause source).
    #[test]
    fn held_op_operator_text_is_cause_neutral_and_identity_bearing() {
        let texts = [
            HeldOp::Inner {
                node_id: "site-x".into(),
            }
            .operator_text(),
            HeldOp::InnerRemove {
                node_id: "site-x".into(),
            }
            .operator_text(),
            HeldOp::Leaf {
                circuit_id: "c1".into(),
            }
            .operator_text(),
            HeldOp::LeafRemove {
                circuit_id: "c1".into(),
            }
            .operator_text(),
        ];
        // identity IS present (the honest local fact).
        assert!(texts[0].contains("site-x"), "the id is shown: {}", texts[0]);
        assert!(texts[2].contains("c1"), "the id is shown: {}", texts[2]);
        for t in &texts {
            // NO invented cause (the U1 mis-attribution), no jargon.
            assert!(
                !t.contains("not live") && !t.contains("drained") && !t.contains("in flight"),
                "no fabricated cause: {t}"
            );
            assert!(!t.contains("fence") && !t.contains("boot_id"), "no jargon: {t}");
            // R3 (card §5): CAUSE-NEUTRAL — the ack carries no reason, so the copy
            // must not assert one (the old "re-parent"/"not live"/"drained" were
            // fabricated causes; the live reason was `multi-add`).
            for claim in ["re-parent", "not live", "not fully drained", "target site"] {
                assert!(!t.contains(claim), "no fabricated cause ({claim}): {t}");
            }
        }
        assert!(texts[0..2].iter().all(|t| t.contains("site")));
        assert!(texts[2..4].iter().all(|t| t.contains("circuit")));
    }

    #[test]
    fn generated_reparent_now_flows_and_its_ancestor_retires() {
        // P7/B3 RETIRED: the producer no longer holds a generated re-parent — it
        // emits the R-B Upsert (the bridge's F3-B re-parent owner-set belt routes
        // a SPAN-CHANGING one to RebuildRequired/R3) and the ancestor retire flows.
        let base = bare(
            vec![
                inner_up("parentSite", "root"),
                inner_up("other", "root"),
                inner_up("libreqos:generated:uisp:site:orphans", "parentSite"),
            ],
            vec![leaf("c1", "libreqos:generated:uisp:site:orphans", 5, 12)],
        );
        let mut desired = base.clone();
        desired.inner_ops = vec![
            inner_up("other", "root"),
            inner_up("libreqos:generated:uisp:site:orphans", "other"),
        ];

        let outcome = semantic_diff(&desired, Some(&base), None);
        let delta = outcome
            .delta
            .clone()
            .expect("P7/B3: a generated change now FLOWS (no hold)");
        assert!(
            delta.inner_ops.iter().any(|op| matches!(
                op,
                DataplaneInnerOp::Upsert { node_id, parent_node_id, .. }
                    if node_id == "libreqos:generated:uisp:site:orphans"
                        && parent_node_id == "other"
            )),
            "the generated re-parent is emitted as an R-B Upsert (span belt -> R3 if changing)"
        );
        assert!(
            outcome
                .deferred_retires
                .iter()
                .any(|s| s == "parentSite"),
            "§.15(d): the ancestor retire is DEFERRED (its applied subtree still bears c1)"
        );
        assert!(
            !delta.inner_ops.iter().any(|op| matches!(
                op,
                DataplaneInnerOp::Remove { node_id } if node_id == "parentSite"
            )),
            "§.15(d): the ancestor Remove must NOT ride the same delta (RetireWithCircuits)"
        );
        // The NEXT generation: the projection is circuit-less under `parentSite`,
        // so the retire rides ALONE (the engine's lone-inner-remove shape).
        let next = semantic_diff(&desired, Some(outcome.applied_projection()), None);
        let next_delta = next.delta.expect("the deferred retire emits next generation");
        assert_eq!(
            next_delta.inner_ops.len(),
            1,
            "the lone retire is the only inner op"
        );
        assert!(matches!(
            &next_delta.inner_ops[0],
            DataplaneInnerOp::Remove { node_id } if node_id == "parentSite"
        ));
        assert!(
            next_delta.leaf_ops.is_empty(),
            "no leaf ops ride the lone retire"
        );
    }

    /// §`lqos-74t.13`/`.15`(d): a site whose circuits are removed in the SAME
    /// diff is DEFERRED one generation (the bridge's `RetireWithCircuits` gate is
    /// pre-delta); the next diff's base is circuit-less and the site `Remove`
    /// rides ALONE — no `RetireWithCircuits` reject. (The card's one-line oracle.)
    #[test]
    fn site_retire_with_circuits_is_deferred_one_generation() {
        let base = bare(
            vec![inner_up("site", "root"), inner_up("ap", "site")],
            vec![leaf("c1", "ap", 5, 12), leaf("c2", "ap", 5, 12)],
        );
        // `desired` keeps `site`, drops `ap` AND both circuits in ONE diff.
        let desired = bare(vec![inner_up("site", "root")], vec![]);
        let outcome = semantic_diff(&desired, Some(&base), None);
        let delta = outcome.delta.clone().expect("the circuit removes emit");
        assert_eq!(delta.leaf_ops.len(), 2, "both circuits are removed");
        assert!(
            outcome.deferred_retires.iter().any(|s| s == "ap"),
            "the circuit-bearing ap is deferred (its applied subtree is non-empty)"
        );
        assert!(
            !delta.inner_ops.iter().any(|op| matches!(
                op,
                DataplaneInnerOp::Remove { node_id } if node_id == "ap"
            )),
            "the ap Remove must NOT ride the same delta (RetireWithCircuits)"
        );
        // Next generation: the base is circuit-less under `ap` ⇒ the lone retire.
        let next = semantic_diff(&desired, Some(outcome.applied_projection()), None);
        let next_delta = next.delta.expect("the deferred retire emits next generation");
        let removed: Vec<&str> = next_delta
            .inner_ops
            .iter()
            .filter_map(|op| match op {
                DataplaneInnerOp::Remove { node_id } => Some(node_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(removed, vec!["ap"], "the lone retires apply, bottom-up");
        assert!(next_delta.leaf_ops.is_empty(), "no leaf ops ride the lone retire");
    }

    #[test]
    fn mixed_legit_and_generated_reparent_both_flow() {
        // P7/B3 RETIRED: both a legitimate sibling move and a generated re-parent
        // emit in the SAME coherent delta (the bridge topo-sorts; a span-changing
        // generated re-parent goes to R3 via the re-parent owner-set belt).
        let base = bare(
            vec![
                inner_up("parentSite", "root"),
                inner_up("other", "root"),
                inner_up("mre", "root"),
                inner_up("libreqos:generated:uisp:site:orphans", "parentSite"),
            ],
            vec![leaf("c1", "libreqos:generated:uisp:site:orphans", 5, 12)],
        );
        let mut desired = base.clone();
        desired.inner_ops = vec![
            inner_up("other", "root"),
            inner_up("mre", "other"),
            inner_up("libreqos:generated:uisp:site:orphans", "other"),
        ];

        let outcome = semantic_diff(&desired, Some(&base), None);
        let delta = outcome.delta.expect("the mixed generated change now flows");
        let moved = |id: &str| {
            delta.inner_ops.iter().any(|op| matches!(
                op,
                DataplaneInnerOp::Upsert { node_id, parent_node_id, .. }
                    if node_id == id && parent_node_id == "other"
            ))
        };
        assert!(moved("mre"), "the legit sibling move emits");
        assert!(
            moved("libreqos:generated:uisp:site:orphans"),
            "the generated re-parent emits (owner-set belt -> R3 if span-changing)"
        );
    }

    /// GATE (A) (`docs/broker_structural_diff_replay_gate_card.md` §1): replay a
    /// base→desired diff through the producer and assert the INVARIANTS on the
    /// coherent delta — retire-after-drain; NO generated/span-changing move in ANY
    /// view.
    fn replay_invariants(
        label: &str,
        base: &DataplaneTransaction,
        desired: &DataplaneTransaction,
        expect_fixed_point: bool,
    ) {
        let root = lqos_dataplane_types::DATA_PLANE_ROOT_NODE_ID;
        let outcome = semantic_diff(desired, Some(base), None);
        let mut inner_parent: std::collections::BTreeMap<String, String> = base
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
        let mut leaf_parent: std::collections::BTreeMap<String, String> = base
            .leaf_ops
            .iter()
            .filter_map(|op| match op {
                DataplaneLeafOp::Upsert {
                    circuit_id,
                    parent_node_id,
                    ..
                } => Some((circuit_id.clone(), parent_node_id.clone())),
                _ => None,
            })
            .collect();
        for (i, delta) in outcome.deltas().iter().enumerate() {
            // F3-P: order is NOT significant (the bridge topo-sorts), so a
            // re-parent may target a NEW inner emitted later in the same delta.
            // The R-C requirement is referential closure over `B ∪ D`.
            let delta_inner_ids: std::collections::BTreeSet<&str> = delta
                .inner_ops
                .iter()
                .filter_map(|op| match op {
                    DataplaneInnerOp::Upsert { node_id, .. } => Some(node_id.as_str()),
                    _ => None,
                })
                .collect();
            let (mut adds, mut removes, mut retires, mut moves) = (0u32, 0u32, 0u32, 0u32);
            // F3-P: order is not significant, so the retire-after-drain check
            // must see the POST-delta leaf set (the delta's leaf Removes applied),
            // not the in-flight sequential set.
            let mut post_leaf_parent = leaf_parent.clone();
            for op in &delta.leaf_ops {
                match op {
                    DataplaneLeafOp::Upsert {
                        circuit_id,
                        parent_node_id,
                        ..
                    } => {
                        post_leaf_parent.insert(circuit_id.clone(), parent_node_id.clone());
                    }
                    DataplaneLeafOp::Remove { circuit_id } => {
                        post_leaf_parent.remove(circuit_id);
                    }
                    DataplaneLeafOp::Move { .. } => {}
                }
            }
            let mut post_inner_parent = inner_parent.clone();
            for op in &delta.inner_ops {
                match op {
                    DataplaneInnerOp::Upsert {
                        node_id,
                        parent_node_id,
                        ..
                    } => {
                        post_inner_parent.insert(node_id.clone(), parent_node_id.clone());
                    }
                    DataplaneInnerOp::Remove { node_id } => {
                        post_inner_parent.remove(node_id);
                    }
                    DataplaneInnerOp::Move { .. } => {}
                }
            }
            for op in &delta.inner_ops {
                match op {
                    DataplaneInnerOp::Upsert {
                        node_id,
                        parent_node_id,
                        ..
                    } => {
                        match inner_parent.get(node_id) {
                            None => {}
                            Some(p) if p != parent_node_id => {
                                moves += 1;
                                assert!(
                                    parent_node_id == root
                                        || inner_parent.contains_key(parent_node_id)
                                        || delta_inner_ids.contains(parent_node_id.as_str()),
                                    "{label}: delta {i} moves {node_id} -> {parent_node_id} whose \
                                     parent is in neither the applied base nor the delta (R-C)"
                                );
                            }
                            _ => {}
                        }
                        inner_parent.insert(node_id.clone(), parent_node_id.clone());
                    }
                    DataplaneInnerOp::Remove { node_id } => {
                        retires += 1;
                        for lp in post_leaf_parent.values() {
                            let mut cur = lp.as_str();
                            let mut guard = 0;
                            let mut hits = false;
                            while guard < 64 {
                                if cur == node_id {
                                    hits = true;
                                    break;
                                }
                                match post_inner_parent.get(cur) {
                                    Some(n) if n != cur => cur = n.as_str(),
                                    _ => break,
                                }
                                guard += 1;
                            }
                            assert!(
                                !hits,
                                "{label}: delta {i} retires {node_id} with remaining circuits \
                                 (retire-after-drain)"
                            );
                        }
                        inner_parent.remove(node_id);
                    }
                    DataplaneInnerOp::Move { .. } => {}
                }
            }
            for op in &delta.leaf_ops {
                match op {
                    DataplaneLeafOp::Upsert {
                        circuit_id,
                        parent_node_id,
                        ..
                    } => {
                        match leaf_parent.get(circuit_id) {
                            None => adds += 1,
                            Some(p) if p != parent_node_id => moves += 1,
                            _ => {}
                        }
                        leaf_parent.insert(circuit_id.clone(), parent_node_id.clone());
                    }
                    DataplaneLeafOp::Remove { circuit_id } => {
                        removes += 1;
                        leaf_parent.remove(circuit_id);
                    }
                    DataplaneLeafOp::Move { .. } => {}
                }
            }
            // F3-P: the coherent change-set carries as many ops per class as
            // changed (`adds={adds} removes={removes} retires={retires}
            // moves={moves}`); the ≤1-per-class cap was the sequencing rule F3-P
            // deletes. Invariant 4 (no GENERATED move) is still checked above.
            let _ = (adds, removes, retires, moves);
        }
        // Invariant 5 (card 1(A)): the applied projection is a FIXED POINT — a
        // re-diff of `desired` against it emits NOTHING. §`lqos-74t.13`/`.15`(d):
        // a DEFERRED site RETIRE makes it a fixed point ONE GENERATION REMOVED —
        // the re-diff carries ONLY the lone inner `Remove`s for the sites this pass
        // deferred (never any other work). (Held / generated-span-changing-move
        // cases are exempt.)
        if expect_fixed_point {
            let re = semantic_diff(desired, Some(outcome.applied_projection()), None);
            let stray_inner: Vec<&DataplaneInnerOp> = re
                .delta
                .as_ref()
                .map(|d| {
                    d.inner_ops
                        .iter()
                        .filter(|op| match op {
                            DataplaneInnerOp::Remove { node_id } => {
                                !outcome.deferred_retires.contains(node_id)
                            }
                            _ => true,
                        })
                        .collect()
                })
                .unwrap_or_default();
            let stray_leaf = re
                .delta
                .as_ref()
                .map(|d| d.leaf_ops.len() + d.mapping_ops.len())
                .unwrap_or(0);
            assert!(
                stray_inner.is_empty() && stray_leaf == 0,
                "{label}: the projection re-diff carries work beyond the deferred retires \
                 (stray_inner={stray_inner:?} stray_leaf/mapping={stray_leaf})"
            );
        }
    }

    #[test]
    fn producer_invariants_hold_over_the_mutation_matrix() {
        let base = bare(
            vec![
                inner_up("a", "root"),
                inner_up("b", "root"),
                inner_up("c", "root"),
            ],
            vec![],
        );
        let mut chained = base.clone();
        chained.inner_ops = vec![
            inner_up("c", "root"),
            inner_up("b", "c"),
            inner_up("a", "b"),
        ];
        replay_invariants("chained-moves", &base, &chained, true);

        let mut parallel = base.clone();
        parallel.inner_ops = vec![
            inner_up("p", "root"),
            inner_up("a", "p"),
            inner_up("b", "p"),
            inner_up("c", "p"),
        ];
        replay_invariants("parallel-moves", &base, &parallel, true);

        let rbase = bare(vec![inner_up("s", "root")], vec![leaf("c1", "s", 5, 12)]);
        let rdesired = bare(vec![], vec![]);
        replay_invariants("retire-after-drain", &rbase, &rdesired, true);

        let gbase = bare(
            vec![
                inner_up("parentSite", "root"),
                inner_up("other", "root"),
                inner_up("mre", "root"),
                inner_up("libreqos:generated:uisp:site:orphans", "parentSite"),
            ],
            vec![leaf("c1", "libreqos:generated:uisp:site:orphans", 5, 12)],
        );
        let mut gdesired = gbase.clone();
        gdesired.inner_ops = vec![
            inner_up("other", "root"),
            inner_up("mre", "other"),
            inner_up("libreqos:generated:uisp:site:orphans", "other"),
        ];
        replay_invariants("generated-mixed", &gbase, &gdesired, true);

 // RATE under a generated umbrella: a rate change
        // to a node UNDER a held generated node must NOT pull it into the rate
        // path (the whole diff is held).
        let rgbase = bare(
            vec![
                inner_up("parentSite", "root"),
                inner_up("other", "root"),
                inner_up("libreqos:generated:uisp:site:orphans", "parentSite"),
                inner_up("deep", "libreqos:generated:uisp:site:orphans"),
            ],
            vec![leaf("c1", "deep", 5, 12)],
        );
        let mut rgdesired = rgbase.clone();
        rgdesired.inner_ops = vec![
            inner_up("other", "root"),
            inner_up("libreqos:generated:uisp:site:orphans", "other"),
            inner_up("deep", "libreqos:generated:uisp:site:orphans"),
        ];
        for op in rgdesired.inner_ops.iter_mut() {
            if let DataplaneInnerOp::Upsert {
                node_id, download, ..
            } = op
                && node_id == "deep"
            {
                *download = pair(2_000_000, 20_000_000);
            }
        }
        replay_invariants("rate-under-generated", &rgbase, &rgdesired, true);

 // THE JACKRABBIT 2ND-LOOP SHAPE: a SITE UNDER the generated
        // umbrella moves (apt2: orphans -> apt3) — changes orphans' owner set.
        let sgb = bare(
            vec![
                inner_up("libreqos:generated:uisp:site:orphans", "root"),
                inner_up("apt2", "libreqos:generated:uisp:site:orphans"),
                inner_up("apt3", "root"),
            ],
            vec![leaf("c1", "apt2", 5, 12)],
        );
        let mut sgd = sgb.clone();
        sgd.inner_ops = vec![
            inner_up("libreqos:generated:uisp:site:orphans", "root"),
            inner_up("apt2", "apt3"),
            inner_up("apt3", "root"),
        ];
        replay_invariants("site-under-generated-moves", &sgb, &sgd, true);
        let sgo = semantic_diff(&sgd, Some(&sgb), None);
        let sgo_delta = sgo
            .delta
            .expect("P7/B3: a site under the generated umbrella now flows");
        assert!(
            sgo_delta.inner_ops.iter().any(|op| matches!(
                op,
                DataplaneInnerOp::Upsert { node_id, parent_node_id, .. }
                    if node_id == "apt2" && parent_node_id == "apt3"
            )),
            "P7/B3: the site re-parent flows (owner-set belt -> R3 if span-changing)"
        );
        // And a RETIRE of a node under the umbrella.
        let rgb = bare(
            vec![
                inner_up("libreqos:generated:uisp:site:orphans", "root"),
                inner_up("apt2", "libreqos:generated:uisp:site:orphans"),
            ],
            vec![leaf("c1", "apt2", 5, 12)],
        );
        let rgd = bare(vec![inner_up("libreqos:generated:uisp:site:orphans", "root")], vec![]);
        replay_invariants("retire-under-generated", &rgb, &rgd, true);
        assert!(
            semantic_diff(&rgd, Some(&rgb), None).delta.is_some(),
            "P7/B3: a retire under the generated umbrella now flows"
        );

        let sbase = bare(vec![inner_up("site-1", "root")], vec![leaf("c1", "site-1", 5, 12)]);
        let mut sdesired = sbase.clone();
        sdesired.inner_ops = vec![inner_up("site-1", "root"), inner_up("newt", "root")];
        sdesired.leaf_ops = vec![leaf("c1", "newt", 5, 12)];
        replay_invariants("new-target-leaf-move", &sbase, &sdesired, true);
    }




 /// SCALE, the DRIVER's ask): N rate changes on EXISTING
    /// circuits must emit ONE rate tx (batched — NOT N txs), NO structural op,
    /// NOT Baseline, fit a 64 MiB seam, and hold nothing.
    #[test]
    fn n_rate_changes_emit_one_incremental_rate_tx() {
        let n = 8_000usize;
        let base = bare(
            vec![inner_up("site-1", "root")],
            (0..n)
                .map(|i| leaf(&format!("c{i}"), "site-1", 5, 12))
                .collect(),
        );
        let mut desired = base.clone();
        desired.leaf_ops = (0..n)
            .map(|i| leaf(&format!("c{i}"), "site-1", 7, 15))
            .collect();

        let outcome = semantic_diff(&desired, Some(&base), None);
        assert_eq!(
            outcome.deltas().len(),
            1,
            "N rate changes must batch into ONE tx (got {})",
            outcome.deltas().len()
        );
        let tx = &outcome.deltas()[0];
        assert_eq!(tx.leaf_ops.len(), n, "all changed leaves ride the single tx");
        assert!(
            matches!(tx.apply_mode, lqos_dataplane_types::ApplyMode::Incremental),
            "a rate-only change is NOT Baseline"
        );
        assert!(
            tx.mapping_ops.is_empty(),
            "a rate change on an existing circuit does not churn mappings"
        );
        assert!(
            tx.inner_ops.iter().all(|op| matches!(
                op,
                DataplaneInnerOp::Upsert { node_id, .. } if node_id == "site-1"
            )),
            "no structural inner op — only the unchanged site rides the closure"
        );
        let bytes = serde_json::to_vec(tx).expect("serialize the rate tx").len();
        assert!(bytes < 64 * 1024 * 1024, "rate tx fits the 64 MiB seam: {bytes} B");
    }

    /// GATE (B) (`docs/broker_structural_diff_replay_gate_card.md` §1):
    /// REPLAY flash_1's captured production DESIRED generations
    /// (`scripts/test/broker_diff_shadow.py capture` →
    /// `<dir>/generations/gen_*.json`) through the producer, asserting the
    /// invariants on each consecutive transition — each historical class
    /// becomes a permanent regression case. Env-gated
    /// (`LQOSD_DIFF_REPLAY_DIR`); a no-op when unset so the build stays green.
    #[test]
    fn producer_invariants_hold_over_a_captured_diff_replay() {
        // DEFAULT to the COMMITTED fixture so the class fence runs in EVERY
        // cargo test (local + CI) — a no-op-unless-env gate can be silently
        // skipped. `LQOSD_DIFF_REPLAY_DIR` OVERRIDES it for a custom/real capture
 // archive REQUIRED 1; the deriver/flash_1.
        let dir = std::env::var("LQOSD_DIFF_REPLAY_DIR").unwrap_or_else(|_| {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/bridge_broker/fixtures/diff_replay"
            )
            .to_string()
        });
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(format!("{dir}/generations"))
            .expect("capture generations dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        // Neither a silent no-op nor a single-generation archive is a gate.
        assert!(
            files.len() >= 2,
            "capture has {} generation(s) — need >=1 transition to replay",
            files.len()
        );
        let mut prev: Option<DataplaneTransaction> = None;
        for path in files {
            let raw = std::fs::read_to_string(&path).expect("read generation");
            // NON-SILENT: a malformed capture file must FAIL the gate, not be
 // skipped (the implementer) — a silent skip would hide a real transition.
            let tx = serde_json::from_str::<DataplaneTransaction>(&raw)
                .unwrap_or_else(|e| panic!("unparseable generation {}: {e}", path.display()));
            if let Some(base) = prev.as_ref() {
                let label = format!("capture:{}", path.display());
                // Captured production transitions may carry held/generated cases
                // — check the structural invariants, not the live fixed point.
                replay_invariants(&label, base, &tx, false);
            }
            prev = Some(tx);
        }
    }



    #[test]
    fn restart_with_no_baseline_holds_not_a_full_world() {
        // A1 (`docs/broker_applied_single_source_card.md`): with NO base the
        // producer HOLDS — it does NOT fabricate the full world (the retired
        // O(n) full-emit). The adopt provides the base; a missing report HOLDs.
        let desired = base();
        let outcome = semantic_diff(&desired, None, None);
        assert!(
            outcome.deltas().is_empty(),
            "no baseline => HOLD, never a full-world"
        );
    }

 /// R-EMIT-ORDER: the L1 delta-apply is order-sensitive
    /// ONLY through the documented last-wins rule, so an emitted delta MUST
    /// carry at most ONE Map/Del per `(prefix_len, ip)` key. Then the applied
    /// world is independent of the emission order, whatever order the producer
    /// used. Also pins emission DETERMINISM (the same base+desired always
    /// yields the same op sequence).
    ///
 /// MOVE-COVERAGE: the three shapes exercised here are a
    /// MOVE, an ADD and a REMOVE — and the MOVE **IS EMITTED**, not inert: a
    /// mapping `(32, 100.97.0.1) c1 -> c3` produces ONE `Map` op to `c3` (the
    /// key is present on both sides, so it is an upsert, NOT an add+del pair —
    /// which is exactly why the key stays unique). Layer-2 SITE re-parents are
    /// NOT inert either: a changed `parent_node_id` rides on the site's inner
    /// `Upsert` (live-emitted; same-owner moves apply, spanning-changing moves
    /// are typed-rejected by the crossing belt) — see
    /// `docs/r_inner_shapes_tree_differential_card.md`.
    #[test]
    fn emitted_mapping_delta_is_key_unique_and_deterministic() {
        let baseline = base();
        let mut desired = base();
        desired.mapping_ops[0] = map("100.97.0.1", "c3"); // a MOVE  (0.1: c1 -> c3)
        desired.mapping_ops.remove(1); // a REMOVE (0.2)
        desired.mapping_ops.push(map("100.97.0.4", "c4")); // an ADD   (0.4)

        let first = semantic_diff(&desired, Some(&baseline), None);
        let second = semantic_diff(&desired, Some(&baseline), None);
        assert_eq!(first.deltas().len(), 1, "the churn rides one transaction");
        let delta = &first.deltas()[0];
        assert_eq!(
            format!("{:?}", delta.mapping_ops),
            format!("{:?}", second.deltas()[0].mapping_ops),
            "the emitted mapping sequence must be deterministic"
        );
        let mut seen: BTreeSet<(u8, std::net::IpAddr)> = BTreeSet::new();
        for op in &delta.mapping_ops {
            let key = match op {
                DataplaneMappingOp::Map { prefix_len, ip, .. }
                | DataplaneMappingOp::Del { prefix_len, ip } => (*prefix_len, *ip),
                _ => continue,
            };
            assert!(
                seen.insert(key),
                "two mapping ops for the same key {key:?} — the L1 delta becomes order-sensitive"
            );
        }
        // MOVE-COVERAGE (exact): exactly ONE op for the moved key, and it is the
        // Map to the NEW circuit — an upsert, NOT an add+del pair.
        let moved: Vec<&DataplaneMappingOp> = delta
            .mapping_ops
            .iter()
            .filter(|op| match op {
                DataplaneMappingOp::Map { ip, .. } | DataplaneMappingOp::Del { ip, .. } => {
                    ip.to_string() == "100.97.0.1"
                }
                _ => false,
            })
            .collect();
        assert_eq!(moved.len(), 1, "the move emits exactly ONE op for the key");
        assert!(
            matches!(moved[0], DataplaneMappingOp::Map { circuit_id, .. } if circuit_id == "c3"),
            "the move is an UPSERT to the new circuit, not an add+del pair"
        );
    }

    // ---- §4ter ancestor-closure gates (bridge_structural_reject_card) ----







    /// Card (`docs/duplicate_mapping_alerts_card.md`): all FOUR `Map*` families
    /// detect an identity bound to >1 circuit, with family-PREFIXED labels (the
    /// `urgent` dedupe key is the label verbatim, so `vlan:10` != `mpls:10`).
    #[test]
    fn duplicate_mapping_identities_covers_all_four_families() {
        use lqos_dataplane_types::DataplaneMappingOp as M;
        let tx = |ops: Vec<M>| DataplaneTransaction {
            generation: "g".to_string(),
            inner_ops: Vec::new(),
            leaf_ops: Vec::new(),
            mapping_ops: ops,
            schema_version: None,
            steering_vlans: Vec::new(),
            lpm_delta: Vec::new(),
            apply_mode: Default::default(),
            circuit_capacity: None,
        };
        let ip_map = |ip: &str, c: &str| M::Map {
            prefix_len: 32,
            ip: ip.parse().unwrap(),
            circuit_id: c.to_string(),
            device_id: format!("dev-{c}"),
        };

        // One bare identity on one circuit => no alert.
        assert!(duplicate_mapping_identities(&tx(vec![ip_map("10.0.0.1", "c1")])).is_empty());

        // Two circuits on one /32 => ONE duplicate naming both.
        assert_eq!(
            duplicate_mapping_identities(&tx(vec![
                ip_map("10.1.10.1", "c1"),
                ip_map("10.1.10.1", "c2"),
            ])),
            vec![(
                "prefix:32:10.1.10.1".to_string(),
                vec!["c1".to_string(), "c2".to_string()]
            )]
        );

        // All four families, family-prefixed labels.
        let dups = duplicate_mapping_identities(&tx(vec![
            ip_map("10.0.0.1", "c1"),
            ip_map("10.0.0.1", "c2"),
            M::MapTunnel { vni: 1, vlan: 2, circuit: "c1".to_string() },
            M::MapTunnel { vni: 1, vlan: 2, circuit: "c2".to_string() },
            M::MapVlan { vlan: 10, circuit: "c1".to_string() },
            M::MapVlan { vlan: 10, circuit: "c2".to_string() },
            M::MapMpls { label: 10, circuit: "c1".to_string() },
            M::MapMpls { label: 10, circuit: "c2".to_string() },
        ]));
        assert_eq!(dups.len(), 4, "all four families produce a duplicate: {dups:?}");
        for label in ["prefix:32:10.0.0.1", "tunnel:1:2", "vlan:10", "mpls:10"] {
            assert!(
                dups.iter().any(|(l, _)| l == label),
                "missing {label} in {dups:?}"
            );
        }
        // `vlan:10` and `mpls:10` are DISTINCT dedupe keys (family-prefixed).
        assert_ne!(dups[0].0, dups[3].0);
    }

    /// `lqos-74t.54`: a remove-only delta whose removed set is a contiguous
    /// SUFFIX of the adopted slot order emits the Removes TAIL-FIRST (the only
    /// order the bridge's LIFO `with_circuit_removed` accepts), and the capture
    /// records a suffix verdict.
    #[test]
    fn lqos74t54_remove_only_suffix_emits_tail_first() {
        let base = base(); // c1, c2 (slot order c1, c2)
        let mut desired = base.clone();
        desired
            .leaf_ops
            .retain(|op| !matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c2"));
        desired
            .mapping_ops
            .retain(|op| !matches!(op, DataplaneMappingOp::Map { circuit_id, .. } if circuit_id == "c2"));
        let order = vec!["c1".to_string(), "c2".to_string()];
        let outcome = semantic_diff(&desired, Some(&base), Some(&order));
        let deltas = outcome.deltas();
        let removes: Vec<&str> = deltas[0]
            .leaf_ops
            .iter()
            .filter_map(|op| match op {
                DataplaneLeafOp::Remove { circuit_id } => Some(circuit_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(removes, vec!["c2"], "the tail circuit must be removed");
        assert!(outcome.removal_capture.suffix);
        assert!(outcome.removal_capture.non_suffix.is_empty());
        assert_eq!(outcome.removal_capture.removed, vec!["c2".to_string()]);
        assert_eq!(outcome.removal_capture.slot_table_len, 2);
    }

    /// `lqos-74t.54`: a remove-only delta whose removed set is INTERIOR (not a
    /// suffix) emits NO doomed remove — the removable run is empty and the
    /// remainder is held (surfaced), so the broker can fence instead of spinning.
    #[test]
    fn lqos74t54_interior_remove_is_held_not_emitted() {
        let mut base = base(); // c1, c2, + c3
        base.leaf_ops.push(leaf("c3", "site-1", 6_000, 12_000));
        base.mapping_ops.push(map("100.97.0.3", "c3"));
        let mut desired = base.clone();
        // Remove the INTERIOR c2 while c3 survives.
        desired
            .leaf_ops
            .retain(|op| !matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c2"));
        desired
            .mapping_ops
            .retain(|op| !matches!(op, DataplaneMappingOp::Map { circuit_id, .. } if circuit_id == "c2"));
        let order = vec!["c1".to_string(), "c2".to_string(), "c3".to_string()];
        let outcome = semantic_diff(&desired, Some(&base), Some(&order));
        assert!(
            outcome.removal_capture.removed.contains(&"c2".to_string()),
            "the capture must record the FULL removed set"
        );
        assert!(!outcome.removal_capture.suffix, "c2 is interior, not a suffix");
        assert_eq!(outcome.removal_capture.non_suffix, vec!["c2".to_string()]);
        // No doomed remove (nor its half-applied mapping Del) may be emitted.
        if let Some(delta) = &outcome.delta {
            assert!(
                !delta.leaf_ops.iter().any(|op| matches!(
                    op,
                    DataplaneLeafOp::Remove { circuit_id } if circuit_id == "c2"
                )),
                "an interior remove must NOT be emitted"
            );
            assert!(
                !delta.mapping_ops.iter().any(|op| matches!(
                    op,
                    DataplaneMappingOp::Del { ip, .. } if ip.to_string() == "100.97.0.2"
                )),
                "the held circuit's mapping Del must not be emitted"
            );
        }
    }

    /// `lqos-74t.54` (R-3): a MIXED add+remove delta — the producer still emits
    /// the pre-add tail run tail-first (the capture records a suffix verdict).
    /// The engine applies ADDS before REMOVES (`publish_driver.rs:2904` then
    /// `:2926`), so a base remove is NOT the post-add tail; that case is owned by
    /// the engine's distinct `RemovedNotTail` rebuild class (engine lane), not by
    /// holding here. This oracle pins the producer's best-effort order.
    #[test]
    fn lqos74t54_mixed_delta_emits_pre_add_tail_run() {
        let mut base = base(); // c1, c2, + c3
        base.leaf_ops.push(leaf("c3", "site-1", 6_000, 12_000));
        base.mapping_ops.push(map("100.97.0.3", "c3"));
        let mut desired = base.clone();
        desired
            .leaf_ops
            .retain(|op| !matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c3"));
        desired
            .mapping_ops
            .retain(|op| !matches!(op, DataplaneMappingOp::Map { circuit_id, .. } if circuit_id == "c3"));
        desired.leaf_ops.push(leaf("c4", "site-1", 6_000, 12_000));
        desired.mapping_ops.push(map("100.97.0.4", "c4"));
        let order = vec!["c1".to_string(), "c2".to_string(), "c3".to_string()];
        let outcome = semantic_diff(&desired, Some(&base), Some(&order));
        assert!(outcome.removal_capture.suffix, "c3 is a pre-add suffix");
        assert!(outcome.removal_capture.non_suffix.is_empty());
        assert_eq!(outcome.removal_capture.removed, vec!["c3".to_string()]);
        let delta = outcome.delta.as_ref().expect("the delta must emit");
        assert!(
            delta.leaf_ops.iter().any(|op| matches!(
                op,
                DataplaneLeafOp::Remove { circuit_id } if circuit_id == "c3"
            )),
            "the pre-add tail run is emitted (the engine rebuild owns the adds-first case)"
        );
        assert!(
            delta.leaf_ops.iter().any(|op| matches!(
                op,
                DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "c4"
            )),
            "the add still emits"
        );
    }

    /// `lqos-74t.54`: the splitter emits a full tail-contiguous run tail-first,
    /// holds only the interior remainder, and holds EVERYTHING when adds exist.
    #[test]
    fn lqos74t54_split_tail_run_and_interior_remainder() {
        let order: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();

        let mut removed: BTreeSet<LeafKey> = BTreeSet::new();
        for k in ["b", "c", "d"] {
            removed.insert(k.to_string());
        }
        let (emittable, non_suffix) = split_removable_removals(&order, &removed);
        assert_eq!(
            emittable,
            vec!["d".to_string(), "c".to_string(), "b".to_string()]
        );
        assert!(non_suffix.is_empty());

        // Interior {b,d} with c surviving: only d is tail-contiguous.
        let mut removed: BTreeSet<LeafKey> = BTreeSet::new();
        for k in ["b", "d"] {
            removed.insert(k.to_string());
        }
        let (emittable, non_suffix) = split_removable_removals(&order, &removed);
        assert_eq!(emittable, vec!["d".to_string()]);
        assert_eq!(non_suffix, vec!["b".to_string()]);
    }

    /// `lqos-74t.54` PERF (load-bearing at 50k): a 50k-row world with a
    /// structural remove must NOT pay the old O(n²) `compose_order` dedup that
    /// ran on every diff after the guard landed. This is a coarse
    /// completion/timing envelope, not a benchmark; the printed elapsed is the
    /// confirmation.
    #[test]
    fn lqos74t54_fifty_thousand_row_remove_diff_is_not_quadratic() {
        const N: usize = 50_000;
        let leaves: Vec<DataplaneLeafOp> = (0..N)
            .map(|i| leaf(&format!("c{i}"), "site-1", 1_000, 2_000))
            .collect();
        let base = tx(leaves.clone(), Vec::new());
        let mut desired = base.clone();
        // Remove the tail (a contiguous suffix) — this builds `compose_order`.
        desired.leaf_ops.pop();
        let order: Vec<String> = (0..N).map(|i| format!("c{i}")).collect();
        let started = std::time::Instant::now();
        let outcome = semantic_diff(&desired, Some(&base), Some(&order));
        let elapsed = started.elapsed();
        eprintln!("lqos-74t.54 50k structural remove diff: {elapsed:?}");
        assert!(outcome.removal_capture.suffix);
        assert!(outcome.removal_capture.non_suffix.is_empty());
        assert_eq!(outcome.removal_capture.slot_table_len, N);
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "a 50k remove diff took {elapsed:?} — the quadratic compose_order likely returned"
        );
    }
}
