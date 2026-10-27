//! Shared lqosd<->bridge dataplane transaction schema (the wire contract).
//!
//! **This crate IS the seam contract** — the world identity (`content_key` /
//! `structural_identity`), the wire types, and the typed reject vocabulary the
//! two ends agree on by construction. Read **`docs/seam_contract.md`** (one
//! page) for the flow, the four rules, and the failure model.
//!
//! Extracted 1:1 from lqos_topology::dataplane_transaction (driver ruling
//! 2026-09-04, this crate is the SINGLE SOURCE OF TRUTH for the
//! transaction schema; the DPDK bridge consumes it via private git dep.
//! Protocol-v2 types (PROTOCOL_VERSION const, FrameBody envelope) land
//! here after the bridge switches to this crate — do not duplicate them.
// Dataplane transaction schema: the lqosd<->bridge wire contract types.
// This is Phase 2b step 2: the compiler produces a **full desired-state**
// transaction (every node/leaf as an Upsert, every device IP as a Map). The
// incremental diff (compare desired vs the bridge's current tree) is a separate
// step layered on top.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::IpAddr;

/// Reserved synthetic root node id the bridge attaches top-level nodes under.
pub const DATA_PLANE_ROOT_NODE_ID: &str = "root";

/// The broker's ACK timeout contract (ms). The bridge derives its
/// `all_world_applied` convergence bound from this value (two thirds), so the
/// bridge fails closed/R3 before the broker can classify a lost ACK as
/// `ACK_SILENCE`. This is a timeout contract, not a wire field.
pub const BROKER_ACK_TIMEOUT_MS: u64 = 30_000;

/// `lqos-74t.3` (DRIVER: `orphans` ONLY): the synthetic no-path umbrella's
/// canonical node id — ALWAYS EMITTED (never elided when childless) so it does
/// not toggle create/remove. ONE HOME: both the production emitter
/// (`lqos_topology::dataplane_compiler`) and the bridge guard's compiler
/// (`transaction_compiler::compiler`) read this same const — no cross-crate
/// literal drift.
pub const ORPHANS_UMBRELLA_NODE_ID: &str = "libreqos:generated:uisp:site:orphans";

/// `.115`: lqosd's bounded raw-queue cache capacity. Single-sourced here so
/// the generated flag manifest tracks the name (these are lqosd-only knobs;
/// the shared crate is the scanned home for cross-surface env names).
pub const ENV_RAW_QUEUE_CACHE_CAP: &str = "LQOS_RAW_QUEUE_CACHE_CAP";
/// `.115`: lqosd's raw-queue stale floor (ms).
pub const ENV_RAW_QUEUE_STALE_MS: &str = "LQOS_RAW_QUEUE_STALE_MS";

pub mod wire;

/// The semantic CONTENT KEY of a transaction — SHA-256 over the sorted
/// per-op CBOR encodings (parsed content, not file bytes; op ORDER is not
/// content — the compiler's ordering may vary without a semantic change).
///
/// WP3 (`docs/wp2_wp3_fence_convergence_and_adopt_card.md` §2): this is the
/// **ONE home** for the key both ends compute. The bridge stamps it into its
/// APPLIED-GENERATION report (`analytics_ipc::StateReport::content_key`) and
/// the broker adopts it directly, so the adopt is a PURE function of the
/// report and the two ends can never drift. The algorithm is the historical
/// `lqosd::bridge_broker::translate::content_key` verbatim (the broker
/// delegates here).
pub fn content_key(tx: &DataplaneTransaction) -> String {
    use serde::Serialize;
    use sha2::{Digest, Sha256};

    fn piece<T: Serialize>(value: &T) -> Option<String> {
        let mut cbor = Vec::new();
        ciborium::into_writer(value, &mut cbor)
            .ok()
            .map(|_| format!("{cbor:02x?}"))
    }

    let mut pieces: Vec<String> = Vec::new();
    pieces.extend(tx.inner_ops.iter().filter_map(piece));
    pieces.extend(tx.leaf_ops.iter().filter_map(piece));
    pieces.extend(tx.mapping_ops.iter().filter_map(piece));
    pieces.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"lqosd-bridge-broker-tx-content");
    hasher.update([0xff_u8]);
    for piece in &pieces {
        hasher.update(piece.as_bytes());
        hasher.update([0xfe_u8]);
    }
    let digest = hasher.finalize();
    format!("tx-{digest:02x}")
}

/// The **structural identity** of a world: the sorted inner `(node_id, parent)`
/// and leaf `(circuit_id, parent)` edges — **RATES IGNORED**. This is the LKG
/// rebuild TRIGGER (`docs/lkg_as_cache_card.md` §2, flash_1's refinement): a
/// STRUCTURAL difference ⇒ rebuild; a rate-only difference ⇒ shape the cache and
/// let the incremental publish apply the rate delta (never an O(world) rebuild
/// for a rate churn). ONE home: the broker's `translate::structural_identity`
/// delegates here, so both ends agree by construction.
pub fn structural_identity(tx: &DataplaneTransaction) -> String {
    use sha2::{Digest, Sha256};

    let mut inners: Vec<(&str, &str)> = tx
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
    inners.sort_unstable();
    let mut leaves: Vec<(&str, &str)> = tx
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
    leaves.sort_unstable();

    let mut hasher = Sha256::new();
    hasher.update(b"lqosd-bridge-broker-structural-identity");
    hasher.update([0xff_u8]);
    for (id, parent) in inners {
        hasher.update(id.as_bytes());
        hasher.update([0u8]);
        hasher.update(parent.as_bytes());
        hasher.update([1u8]);
    }
    hasher.update([2u8]);
    for (id, parent) in leaves {
        hasher.update(id.as_bytes());
        hasher.update([0u8]);
        hasher.update(parent.as_bytes());
        hasher.update([1u8]);
    }
    let digest = hasher.finalize();
    format!("struct-{digest:02x}")
}

/// A directional commit/ceil pair in whole Kbps (the BRIDGE wire's
/// native unit — protocol-v2 §8b ruling, driver-confirmed
/// option (a): the crate adopts kbps and the product compiler converts
/// x1000 at emit; one crate means the units can never diverge again).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataplaneRatePair {
    /// Guaranteed minimum rate (HTB `rate`), kbps.
    pub commit_kbps: u32,
    /// Maximum rate including borrowing (HTB `ceil`), kbps.
    pub ceil_kbps: u32,
}

/// `lqos-74t.30` (encode-once): the trickle a suspended (or whole-Mbps-zeroed)
/// circuit is floored to -- 1024 kbps. Carves ICMP/DNS/small HTTP so a
/// suspended customer still reaches a captive portal (the F-E3 bench: 1 kbps
/// was a blackhole; the portal sim passed at 1000 kbps). ONE HOME: both
/// compilers (`lqos_topology::dataplane_compiler` + `transaction_compiler`) and
/// the broker read this, so the suspend encoding can never drift across the
/// seam. The suspend SENTINEL predicate itself lives in
/// `lqos_overrides::CircuitSpeedOverride::is_suspend_floor` (the `0.01` Mbps
/// `SUSPEND_FLOOR_MBPS` HOME) -- not duplicated here.
pub const SUSPEND_RATE_KBPS: u32 = 1024;

/// `lqos-74t.30` (encode-once): floor a direction pair so it can NEVER be
/// `0/0`. A `0/0` pair is `is_disabled()` == FULL line rate at the bridge, so
/// the product compiler's whole-Mbps rounding (`0.01.round() == 0`, and
/// likewise ANY sub-0.5 Mbps config) would silently UNLIMIT the circuit. The
/// suspend sentinel and any other sub-0.5 pair therefore floor to
/// `SUSPEND_RATE_KBPS`. Idempotent: a pair with a nonzero side is unchanged.
///
/// PAIR-LEVEL on purpose: the two compilers build the pair with DIFFERENT
/// helpers (`dataplane_compiler::rate_pair(min_mbps: f32, max_mbps: f32)` vs
/// `transaction_compiler::compiler::rate_pair(commit: u32, ceil: u32)`), so the
/// SHARED unit is the `DataplaneRatePair` itself -- one home both apply at their
/// leaf rate construction, or `assert_analysis_emission_equality` FATALs at
/// boot (`AnalysisEmissionDivergence`).
pub fn floor_suspended_pair(pair: &mut DataplaneRatePair) {
    if pair.commit_kbps == 0 && pair.ceil_kbps == 0 {
        pair.commit_kbps = SUSPEND_RATE_KBPS;
        pair.ceil_kbps = SUSPEND_RATE_KBPS;
    }
}

/// `lqos-74t.15.2` follow-on (inner-commit axis, DRIVER ruling):
/// the **ONE** derivation of an INNER node's directional rate pair from the
/// effective-network artifact. `ceil_mbps` is the node's `*BandwidthMbps`;
/// `min_mbps` is its explicit `*BandwidthMbpsMin` floor (`0` when the artifact
/// omits it — an inner node with no explicit guarantee). BOTH the production
/// emitter (`lqos_topology::dataplane_compiler::node_rate_pair`) and the guard
/// compiler (`transaction_compiler::compiler`, `RateSemantics::Explicit`) call
/// THIS, so the inner commit is a one-home derivation and cannot drift across
/// the seam again (the class the A1 gap belonged to).
///
/// It mirrors the LIVE arithmetic **exactly** (pre-code pin 1):
/// integer Mbps `saturating_mul(1000)`, u32-capped, with **NO floor and NO
/// `commit <= ceil` clamp**. A present `0` Min emits `0/ceil`; a Min greater
/// than the ceiling emits `commit > ceil` — both BY DESIGN, matching the live
/// applied world (adding a floor/clamp here would break the cross-emitter
/// parity the moment such a node appears).
pub fn inner_node_rate_pair(ceil_mbps: u64, min_mbps: u64) -> DataplaneRatePair {
    fn kbps(mbps: u64) -> u32 {
        mbps.saturating_mul(1000).min(u64::from(u32::MAX)) as u32
    }
    DataplaneRatePair {
        commit_kbps: kbps(min_mbps),
        ceil_kbps: kbps(ceil_mbps),
    }
}

/// The A1 GENERAL childless/circuit-less "Unit" rule — **ONE HOME**.
///
/// A node with **NO children** in the effective tree AND **no circuit
/// parented at it** is load-bearing and MUST be kept. This is the analysis
/// predicate (`transaction_compiler::analysis`: `node.children.is_empty() &&
/// !circuits_by_parent.contains_key(&node.id)`) factored into a single
/// helper, so the analysis and BOTH compilers
/// (`lqos_topology::dataplane_compiler::circuit_bearing_nodes` and
/// `transaction_compiler::compiler::circuit_bearing_nodes`) apply the SAME
/// rule. It SUPERSEDES the former `orphans`-only exemption, which it
/// subsumes.
///
/// ONE HOME on purpose: the production emitter and the guard compiler are
/// separate workspaces, so the predicate is shared here rather than
/// re-derived in each. A fork would drop such a node on one side only and the
/// boot `assert_analysis_emission_equality` would FATAL
/// (`AnalysisEmissionDivergence`).
///
/// Parameters mirror the predicate's own terms (`is_childless`,
/// `has_circuit_at_node`) so call sites read exactly as the rule states.
pub fn is_childless_circuitless_unit(is_childless: bool, has_circuit_at_node: bool) -> bool {
    is_childless && !has_circuit_at_node
}

#[cfg(test)]
mod inner_commit_tests {
    use super::*;

    /// The exact live arithmetic: integer Mbps x 1000, no floor, no clamp.
    #[test]
    fn inner_node_rate_pair_mirrors_the_live_arithmetic() {
        // ordinary floor
        assert_eq!(
            inner_node_rate_pair(200, 40),
            DataplaneRatePair {
                commit_kbps: 40_000,
                ceil_kbps: 200_000
            }
        );
        // absent Min reads as 0 (the artifact omission)
        assert_eq!(
            inner_node_rate_pair(1000, 0),
            DataplaneRatePair {
                commit_kbps: 0,
                ceil_kbps: 1_000_000
            }
        );
        // a Min ABOVE the ceiling stays ABOVE (no clamp) — by design
        assert_eq!(
            inner_node_rate_pair(100, 300),
            DataplaneRatePair {
                commit_kbps: 300_000,
                ceil_kbps: 100_000
            }
        );
        // u32 cap
        assert_eq!(inner_node_rate_pair(0, 0).ceil_kbps, 0);
    }
}

#[cfg(test)]
mod suspend_floor_tests {
    use super::*;

    /// §`lqos-74t.30`: a `0/0` pair (the compiled suspend sentinel, or any
    /// sub-0.5 Mbps config) floors to the trickle -- never `is_disabled()` ==
    /// full line rate. Idempotent; a nonzero pair is untouched.
    #[test]
    fn floor_suspended_pair_maps_zero_to_the_trickle_and_is_idempotent() {
        let mut pair = DataplaneRatePair {
            commit_kbps: 0,
            ceil_kbps: 0,
        };
        floor_suspended_pair(&mut pair);
        assert_eq!(
            (pair.commit_kbps, pair.ceil_kbps),
            (SUSPEND_RATE_KBPS, SUSPEND_RATE_KBPS)
        );
        floor_suspended_pair(&mut pair); // idempotent
        assert_eq!(
            (pair.commit_kbps, pair.ceil_kbps),
            (SUSPEND_RATE_KBPS, SUSPEND_RATE_KBPS)
        );

        let mut nonzero = DataplaneRatePair {
            commit_kbps: 1,
            ceil_kbps: 2,
        };
        floor_suspended_pair(&mut nonzero);
        assert_eq!(
            (nonzero.commit_kbps, nonzero.ceil_kbps),
            (1, 2),
            "a nonzero pair is untouched"
        );
    }
}

/// A leaf circuit's AQM policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataplaneLeafAqm {
    /// No per-circuit override (bridge uses its process default).
    None,
    /// fq_codel.
    FqCodel,
    /// CAKE, optionally diffserv4 and an RTT override.
    Cake {
        /// Whether to use CAKE diffserv4 classification.
        diffserv4: bool,
        /// Optional RTT override in milliseconds.
        rtt_ms: Option<u32>,
    },
}

/// An inner-node (site/tower) operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DataplaneInnerOp {
    /// Add the node, or update its rates if it already exists.
    Upsert {
        /// Stable node id.
        node_id: String,
        /// Parent node id (or the synthetic root).
        parent_node_id: String,
        /// Download commit/ceil.
        download: DataplaneRatePair,
        /// Upload commit/ceil.
        upload: DataplaneRatePair,
    },
    /// Remove the node (must be empty).
    Remove {
        /// Stable node id.
        node_id: String,
    },
    /// Reparent the node (and its subtree).
    Move {
        /// Stable node id.
        node_id: String,
        /// New parent node id.
        new_parent_node_id: String,
    },
}

/// A leaf-circuit operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DataplaneLeafOp {
    /// Add the circuit, or update its rates/AQM if it already exists.
    Upsert {
        /// Stable circuit id.
        circuit_id: String,
        /// Parent node id.
        parent_node_id: String,
        /// Download commit/ceil.
        download: DataplaneRatePair,
        /// Upload commit/ceil.
        upload: DataplaneRatePair,
        /// Leaf AQM policy.
        aqm: DataplaneLeafAqm,
        /// Admin enable/disable.
        admin_up: bool,
    },
    /// Remove the circuit.
    Remove {
        /// Stable circuit id.
        circuit_id: String,
    },
    /// Reparent the circuit onto a different node.
    Move {
        /// Stable circuit id.
        circuit_id: String,
        /// New parent node id.
        new_parent_node_id: String,
    },
}

/// CBOR codec discipline (protocol-v2 card §10.7, struct-only model):
/// `std::net::IpAddr`'s serde impl does not round-trip under CBOR
/// (newtype-variant map out, enum expect in — probe-proven). The wire
/// carries the dotted-quad STRING form both ways — identical to the
/// §3.7 JSON fixtures. Ported verbatim from the bridge mirror
/// (rob-dpdk bridge_rs transaction.rs).
pub(crate) mod ip_as_string {
    use std::net::IpAddr;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(ip: &IpAddr, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&ip.to_string())
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<IpAddr, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// An IP→circuit mapping operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DataplaneMappingOp {
    /// Map a prefix to a circuit/device. `prefix_len` is the raw CIDR prefix
    /// (IPv4 `/p` becomes `/96+p` on the bridge's LPM).
    Map {
        /// Raw CIDR prefix length.
        prefix_len: u8,
        /// Network address (dotted-quad/hex string on the wire).
        #[serde(with = "ip_as_string")]
        ip: IpAddr,
        /// Stable circuit id.
        circuit_id: String,
        /// Stable device id.
        device_id: String,
    },
    /// Remove the exact prefix mapping.
    Del {
        /// Raw CIDR prefix length.
        prefix_len: u8,
        /// Network address (dotted-quad/hex string on the wire).
        #[serde(with = "ip_as_string")]
        ip: IpAddr,
    },
    /// Map a tunnel `(VNI, VLAN)` identity to a circuit (T-arc §2.5). The wire
    /// `circuit` is a stable id, resolved to a dense slot at apply time.
    MapTunnel {
        /// 24-bit VXLAN VNI.
        vni: u32,
        /// Inner VLAN id; `u16::MAX` is the untagged/absent sentinel.
        vlan: u16,
        /// Stable circuit id.
        circuit: String,
    },
    /// Remove the exact tunnel identity mapping.
    DelTunnel {
        /// 24-bit VXLAN VNI.
        vni: u32,
        /// Inner VLAN id.
        vlan: u16,
    },
    /// Phase C S2: map a service VLAN id (the FIRST non-steering 802.1Q
    /// tag) to a circuit. The steering tags are excluded by the bridge's
    /// own config, never classified.
    MapVlan {
        /// Service VLAN id (1..=4094; the untagged sentinel is not valid
        /// here — that is the `MapTunnel` vxlan-only encoding).
        vlan: u16,
        /// Stable circuit id.
        circuit: String,
    },
    /// Remove the exact VLAN mapping.
    DelVlan {
        /// Service VLAN id.
        vlan: u16,
    },
    /// Phase C S2: map a top-of-stack MPLS label to a circuit (v1).
    MapMpls {
        /// 20-bit top-of-stack label value.
        label: u32,
        /// Stable circuit id.
        circuit: String,
    },
    /// Remove the exact MPLS-label mapping.
    DelMpls {
        /// 20-bit top-of-stack label value.
        label: u32,
    },
    /// Remove all mappings (the IP table, the tunnel table, the VLAN
    /// table, and the MPLS table).
    Clear,
}

/// How the receiver must apply a transaction
/// (`docs/lossless_auto_reload_impl_work_order.md`, shape M-A).
///
/// `Incremental` (the default) is the existing live path: a structural change
/// the incremental route cannot express is a LOUD typed reject (the
/// `REJECTED_STRUCTURAL_OP` fence — the SAFETY, unchanged). `Baseline` is the
/// opt-in M-A route: the transaction is the COMPLETE desired world and the
/// receiver applies it through its full-world install, bypassing the
/// incremental-only checks. Additive + `#[serde(default)]` ⇒ an OLD receiver
/// reads `Incremental` and a `Baseline` degrades to the class-(A) reject (the
/// intended mixed-version fail-safe); no protocol-version bump.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyMode {
    /// Incremental / partial apply — the existing live path (default).
    #[default]
    Incremental,
    /// The transaction is the COMPLETE desired world; apply it via the
    /// full-world (baseline reset) install. C-B: the arming arm is RETIRED
    /// (no `Baseline` tx is constructed); the variant is kept for wire stability.
    Baseline,
}

/// A versioned dataplane transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataplaneTransaction {
    /// Version hash of the shaping inputs this transaction was compiled from.
    pub generation: String,
    /// Inner-node operations (applied post-order on Remove).
    pub inner_ops: Vec<DataplaneInnerOp>,
    /// Leaf-circuit operations.
    pub leaf_ops: Vec<DataplaneLeafOp>,
    /// IP mapping operations (applied atomically).
    pub mapping_ops: Vec<DataplaneMappingOp>,
    /// STAGE 2 (R-CHANGESET-CARRIER): the EMITTED LPM CHANGE-SET — the
    /// differing `(prefix_len, ip)` identities for THIS step (an ADD's Maps /
    /// a REMOVE's Dels), an OPTIMIZATION HINT only. `mapping_ops` stays the
    /// AUTHORITATIVE full content (multiple consumers + the base-absent
    /// escape); the bridge patches its held applied base with this delta IFF
    /// the base is present AND this is non-empty AND the churn is at/below the
    /// shared crossover, else it takes the existing full build. Additive +
    /// `#[serde(default)]` ⇒ old producers/receivers interoperate unchanged
    /// (no protocol-version bump).
    #[serde(default)]
    pub lpm_delta: Vec<DataplaneMappingOp>,
    /// Phase C S2: the operator's declared steering VLANs (ADVISORY). The
    /// bridge's own resolved stick tags are AUTHORITATIVE for classification;
    /// this declaration is reconciled against them and a mismatch is a loud
    /// warn, never a silent behavior change. Empty = no declaration.
    #[serde(default)]
    pub steering_vlans: Vec<u16>,
    /// Wire schema version (config-surface card M1, protocol-v2 closure 3):
    /// DERIVED from [`wire::PROTOCOL_VERSION`] — one source, the compiler
    /// emits it via this const. `None` = a pre-versioning producer
    /// (serde default keeps legacy fixtures parsing); `Some(v)` with
    /// `v != PROTOCOL_VERSION` is a typed reject / startup FATAL — never
    /// applied. Provenance: the bridge mirror carried this field first
    /// (rob-dpdk 2ac3fc4); the crate is now the single home.
    #[serde(default)]
    pub schema_version: Option<u32>,
    /// M-A (`docs/lossless_auto_reload_impl_work_order.md`): how the receiver
    /// applies this transaction. `Incremental` (the default — and what an old
    /// receiver reads when the field is absent) is the existing live path;
    /// `Baseline` marks a COMPLETE-world emission for the full-world install.
    /// Additive + `#[serde(default)]` ⇒ old producers/receivers interoperate
    /// unchanged (no protocol-version bump).
    #[serde(default)]
    pub apply_mode: ApplyMode,
    /// §8 (`docs/lqos_conf_single_source_card.md`): the DERIVED circuit-slot
    /// capacity for THIS world — `max(next_power_of_two(n_circuits), 1000,
    /// carried)`. The EMITTER (lqosd) sets it; the bridge THREADS it as the
    /// one capacity value (placement input == `PublishContent.capacity` == the
    /// carried seed == the LKG). Additive (`#[serde(default)]`, `None` = a
    /// pre-§8 producer). The LKG stores the transaction, so it inherits this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circuit_capacity: Option<usize>,
}

// ---------------------------------------------------------------------------
// The L1 COW-delta-apply CROSSOVER — the source lever's payload-shape boundary.
//
// It lives HERE, in the shared crate, because the decision must be ONE
// definition read by BOTH ends: the PRODUCER (the broker, which sees the TOTAL
// churn and so chooses the payload shape) and the CONSUMER (the bridge, which
// holds the applied base and patches it). The bridge depends on this crate, so
// the two can never drift.
// ---------------------------------------------------------------------------

/// The COW-delta-apply crossover **N\*** AT THE 50k DESIGN POINT: the churn size
/// above which a full world rebuild is cheaper than N single-op COW applies.
/// For another world size use [`delta_apply_cow_crossover_ops`].
///
/// This is the BROKER's payload-shape boundary: `churn_ops <= N*` ⇒ the
/// true-incremental lone-op sequence; `churn_ops > N*` ⇒ ONE full-world
/// transaction. (Only the producer sees the total churn; a consumer receiving
/// one op per tx can never make this call. The consumer still uses the same
/// constant as a SAFETY bound — a delta whose op count exceeds N\* triggers a
/// rebuild rather than a patch.)
///
/// Measured (one process, both sides, the same 50k world): the rebuild is
/// ~12–22 ms and a COW apply ~0.5–0.9 µs ⇒ the measured crossover is
/// ~20k–33k ops; this value is dated BELOW the measured minimum. The formula
/// and its caveats live on [`delta_apply_cow_crossover_ops`].
///
/// UNITS: **ops**, one op = one prefix insert OR one remove. A *changed* prefix
/// costs up to two ops, so compare a churn's OP count.
pub const DELTA_APPLY_COW_CROSSOVER_OPS: usize = 8_473;

/// N\* for a world of `world` prefixes: **`world^0.9 / 2`**, a SUB-linear form.
///
/// Measured `N* / world` over four world sizes (one process, several runs):
///   ≈ 0.73–0.79 @10k · 0.26–0.47 @50k · 0.27–0.32 @100k · 0.20–0.23 @200k
/// ⇒ N\* is SUB-linear: it falls with the world (fixed overhead in the
/// small-world rebuild) and then flattens; the COW per-op cost is ~flat
/// (0.5–0.9 µs). The 200k point falsified every LINEAR form — `world/4`
/// (50 000) and `world/5` (40 000) both sit at or above the measured
/// 39.3k–45.3k — so they are non-conservative. `α = 0.9` tracks a flattening
/// curve; `k = 1/2` sets the level. The result is below the measured curve at
/// EVERY measured size with margin: 10k 1 990 (3.7×) · 50k 8 473 (1.5×) ·
/// 100k 15 811 (1.7×) · 200k 29 505 (1.3×).
///
/// **STOPGAP**: this is a FOUR-POINT FIT whose tightest margin is at its
/// largest measured size ⇒ beyond 200k it is an extrapolation. Re-validate at
/// any new world size (the calibration bench asserts conservatism per size),
/// and prefer the runtime per-world calibration
/// (`N* = measured_rebuild_ns / measured_per_op_ns`) where available.
pub fn delta_apply_cow_crossover_ops(world: usize) -> usize {
    if world == 0 {
        return 0;
    }
    ((world as f64).powf(0.9) / 2.0).max(1.0) as usize
}

/// The SEQUENCING FLOOR: the minimum churn (in single ops) at which ONE
/// full-world payload may replace the lone-op sequence, whatever the crossover
/// says.
///
/// The crossover is calibrated from 10k prefixes UP, so below that range it
/// extrapolates to a degenerate value (a 2-prefix world gives `N* = 1`, i.e.
/// "always emit the full world"). For a world that small the cost argument is
/// not the deciding factor, and the bounded per-step install + the drain
/// invariants are worth keeping — a NON-COST floor. This floor IS the legacy
/// fixed threshold, retained as a FLOOR rather than the decision.
///
/// It is shared for the same reason the crossover is: the PRODUCER (which
/// chooses the payload shape) and the CONSUMER (which decides whether to patch
/// the delta it is handed) must not disagree — otherwise the producer carefully
/// sequences a tiny-world churn and the consumer rebuilds it anyway
/// (R-FLOOR-PARITY).
pub const SEQUENCE_FLOOR_OPS: usize = 8;

/// The payload-shape threshold for a world of `world` prefixes: the
/// [`delta_apply_cow_crossover_ops`] crossover, floored at
/// [`SEQUENCE_FLOOR_OPS`]. This is THE decision boundary for both ends.
pub fn bulk_shape_threshold_ops(world: usize) -> usize {
    delta_apply_cow_crossover_ops(world).max(SEQUENCE_FLOOR_OPS)
}

/// The override-aware payload-shape threshold — the SINGLE SOURCE OF TRUTH
/// for BOTH ends. `override_ops` is the operator's explicit
/// `LIBREQOS_DPDK_BULK_GENERATION_THRESHOLD` (`Some` when set), which REPLACES
/// the world scaling (neither a floor nor a ceiling); `None` ⇒ the world-scaled
/// [`bulk_shape_threshold_ops`].
pub fn bulk_shape_threshold_for(world: usize, override_ops: Option<usize>) -> usize {
    match override_ops {
        Some(ops) => ops.max(1),
        None => bulk_shape_threshold_ops(world),
    }
}

/// Whether a churn of `churn_ops` single-op applies against a world of `world`
/// prefixes is better served by ONE full-world transaction (`true`) than by the
/// true-incremental lone-op sequence (`false`).
///
/// The producer asks this to pick the payload shape; the consumer asks it to
/// decide whether to PATCH the delta it was handed or REBUILD. `world` must be
/// the SAME quantity on both sides — the world the churn lands on (R-WORLD-PARITY).
pub fn churn_is_full_world(world: usize, churn_ops: usize) -> bool {
    churn_is_full_world_for(world, churn_ops, None)
}

/// The OVERRIDE-AWARE [`churn_is_full_world`].
///
/// R-OVERRIDE-PARITY: the producer (payload-shape choice) and the consumer
/// (patch-or-rebuild) MUST pass the SAME `override_ops`. A producer-side
/// override the consumer ignores makes the producer emit a DELTA that the
/// consumer calls a full world — and a from-payload rebuild of a delta is the
/// DECIMATION class (`docs/jackrabbit_lpm_decimation_card.md` §7 NOTE (2)).
pub fn churn_is_full_world_for(
    world: usize,
    churn_ops: usize,
    override_ops: Option<usize>,
) -> bool {
    churn_ops > bulk_shape_threshold_for(world, override_ops)
}


/// The env override for the **ONE shared size bound K** — the P1 sequence cap
/// and the Baseline size gate (DRIVER-owned). Read by BOTH ends (the bridge's
/// Baseline gate and the broker's routing) so they can never disagree.
pub const BASELINE_MAX_CIRCUITS_ENV: &str = "LIBREQOS_DPDK_BASELINE_MAX_CIRCUITS";

/// The measured per-circuit full-world swap pause (µs) — the Shape-1 coefficient
/// (`docs/o1_pause_line_work_order.md`; ~762 µs/circuit at 926).
pub const BASELINE_US_PER_CIRCUIT: u64 = 762;

/// The acceptable Baseline pause (ms) — the DRIVER's budget.
pub const DEFAULT_BASELINE_MAX_PAUSE_MS: u64 = 50;

/// The **ONE shared circuit bound K**: the measured pause budget
/// (`pause_ms*1000 / us_per_circuit` ≈ 65 at the defaults). Below/at K the broker
/// SEQUENCES churn and the Baseline route may arm; above K the full-world swap is
/// declined. Env-overridable via [`BASELINE_MAX_CIRCUITS_ENV`]. Pure over the
/// lookup so both ends unit-test the same value.
pub fn baseline_max_circuits_from(mut get: impl FnMut(&str) -> Option<String>) -> usize {
    get(BASELINE_MAX_CIRCUITS_ENV)
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or_else(|| {
            (DEFAULT_BASELINE_MAX_PAUSE_MS.saturating_mul(1000)
                / BASELINE_US_PER_CIRCUIT.max(1)) as usize
        })
        .max(1)
}

/// The live leaf rows of a world: `(leaf_name, parent_node_id, worker_owner)`.
/// The owner is the PLACEMENT worker id — supplied by the caller (the engine
/// from its `CircuitOwnership`, the control-plane broker from the applied
/// report). SHARED so the owner-set predicates cannot fork (R-parity): the
/// engine's reparent belt and the broker's maintenance-window twin call the
/// SAME functions.
pub type LeafRow = (String, String, u16);

/// Computes per-node descendant-owner sets over the leaf rows: every leaf
/// contributes its owner to every node on its ancestor chain (root included).
/// Deterministic (BTree) for the crossing comparison.
pub fn compute_owner_sets(
    rows: &[LeafRow],
    inner_parents: &HashMap<String, String>,
) -> BTreeMap<String, BTreeSet<u16>> {
    let mut sets: BTreeMap<String, BTreeSet<u16>> = BTreeMap::new();
    for (_name, parent, owner) in rows {
        let mut cur: &str = parent.as_str();
        // Depth belt: the applied tx is child-before-parent acyclic; the cap
        // turns any future pathology into truncation, not a hang.
        for _ in 0..64 {
            sets.entry(cur.to_string()).or_default().insert(*owner);
            match inner_parents.get(cur) {
                Some(next) if next.as_str() != cur => cur = next.as_str(),
                _ => break,
            }
        }
    }
    sets
}

/// The crossing set: nodes spanning MORE THAN ONE owner. Membership invariance
/// across accepted ops is the S3a proof obligation (the carried bundle's
/// crossings stay exact iff this set never changes).
pub fn crossing_nodes(sets: &BTreeMap<String, BTreeSet<u16>>) -> BTreeSet<String> {
    sets.iter()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(node, _)| node.clone())
        .collect()
}

/// N1 policy-3 comparison, CONTENT-exact: returns the offending node when the
/// successor owner-set map violates "owner sets never change". Every node
/// present in BOTH maps must hold an IDENTICAL set (a name-membership
/// comparison would miss an already-crossing ancestor's set GROWING
/// `{w0,w1}`→`{w0,w1,w2}` at ≥3 workers); a node only in the successor (a
/// payload-created site) must be born single-owner.
pub fn policy3_owner_set_violation(
    current: &BTreeMap<String, BTreeSet<u16>>,
    successor: &BTreeMap<String, BTreeSet<u16>>,
) -> Option<String> {
    for (node, set) in successor {
        match current.get(node) {
            Some(prior) if prior == set => {}
            Some(_) => return Some(node.clone()),
            None if set.len() > 1 => return Some(node.clone()),
            None => {}
        }
    }
    for node in current.keys() {
        if !successor.contains_key(node) {
            return Some(node.clone());
        }
    }
    None
}

/// The process-env [`baseline_max_circuits_from`].
pub fn baseline_max_circuits() -> usize {
    baseline_max_circuits_from(|key| std::env::var(key).ok())
}

/// The consumer-side mirror of the crossover: how a receiver should apply what
/// it is handed — patch the delta, or rebuild the world.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeltaApplyPlan {
    /// O(delta × depth) COW applies — the fast path.
    Cow,
    /// O(world) rebuild — cheaper ABOVE the crossover, or with no base, or for
    /// a full-world payload.
    Rebuild,
}

impl DeltaApplyPlan {
    /// Picks the plan for a churn of `churn_ops` single-op applies (an insert
    /// or a remove) against a world of `world` prefixes. Exactly at the
    /// threshold the COW delta still wins (the rebuild's fixed O(world) term
    /// dominates only strictly above it). Uses the SAME
    /// [`churn_is_full_world`] boundary as the producer, floor included.
    pub fn for_churn(world: usize, churn_ops: usize) -> Self {
        if churn_is_full_world(world, churn_ops) {
            Self::Rebuild
        } else {
            Self::Cow
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::PROTOCOL_VERSION;

    /// R-OVERRIDE-PARITY (`docs/jackrabbit_lpm_decimation_card.md` §7 NOTE
    /// (2)): the override-aware threshold is the SINGLE SOURCE OF TRUTH — the
    /// producer's shape choice and the consumer's patch-or-rebuild decision
    /// move TOGETHER, in both directions.
    #[test]
    fn override_parity_moves_both_ends_together() {
        let world = 50_000usize;
        let n_star = bulk_shape_threshold_ops(world);
        // No override ⇒ the world-scaled crossover (the shared default).
        assert_eq!(bulk_shape_threshold_for(world, None), n_star);
        assert!(!churn_is_full_world_for(world, n_star, None));
        assert!(churn_is_full_world_for(world, n_star + 1, None));
        assert_eq!(
            churn_is_full_world(world, n_star + 1),
            churn_is_full_world_for(world, n_star + 1, None),
            "the legacy 2-arg form IS the no-override case"
        );
        // An override ABOVE N* SEQUENCES the over-N* churn: BOTH ends see the
        // raised boundary — the parity whose absence made the consumer call a
        // DELTA a full world (the decimation class).
        let raised = n_star + 5;
        assert_eq!(bulk_shape_threshold_for(world, Some(raised)), raised);
        assert!(!churn_is_full_world_for(world, n_star + 1, Some(raised)));
        assert!(churn_is_full_world_for(world, raised + 1, Some(raised)));
        // An override BELOW N* makes MORE churn take the full-world path.
        assert!(churn_is_full_world_for(world, n_star, Some(2)));
        // The override is never 0 (clamped to 1).
        assert_eq!(bulk_shape_threshold_for(world, Some(0)), 1);
    }

    /// T-arc §2.5 / Phase C S2: the tunnel + vlan + mpls mapping ops
    /// round-trip with their external tags, and `PROTOCOL_VERSION` moved to 4
    /// WITH them (one commit — the bump and the variants are never split).
    #[test]
    fn tunnel_mapping_ops_round_trip() {
        let ops = vec![
            DataplaneMappingOp::MapTunnel {
                vni: 100,
                vlan: 64,
                circuit: "c1".to_string(),
            },
            DataplaneMappingOp::DelTunnel { vni: 100, vlan: 64 },
            DataplaneMappingOp::MapVlan {
                vlan: 64,
                circuit: "c1".to_string(),
            },
            DataplaneMappingOp::DelVlan { vlan: 64 },
            DataplaneMappingOp::MapMpls {
                label: 1_048_575,
                circuit: "c2".to_string(),
            },
            DataplaneMappingOp::DelMpls { label: 1_048_575 },
            DataplaneMappingOp::Clear,
        ];
        let json = serde_json::to_string(&ops).unwrap();
        assert!(json.contains("\"op\":\"map_tunnel\""), "{json}");
        assert!(json.contains("\"op\":\"del_tunnel\""), "{json}");
        assert!(json.contains("\"op\":\"map_vlan\""), "{json}");
        assert!(json.contains("\"op\":\"map_mpls\""), "{json}");
        let back: Vec<DataplaneMappingOp> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ops);
        assert_eq!(PROTOCOL_VERSION, 6);
    }

    /// Phase C S2: `steering_vlans` is advisory, serde-defaults empty, and
    /// round-trips.
    #[test]
    fn steering_vlans_defaults_empty_and_round_trips() {
        let tx: DataplaneTransaction = serde_json::from_str(
            r#"{"generation":"g","inner_ops":[],"leaf_ops":[],"mapping_ops":[]}"#,
        )
        .expect("legacy fixture parses");
        assert!(tx.steering_vlans.is_empty(), "serde default = empty");
        let tx = DataplaneTransaction {
            steering_vlans: vec![10, 20],
            ..tx
        };
        let json = serde_json::to_string(&tx).unwrap();
        assert!(json.contains("\"steering_vlans\":[10,20]"), "{json}");
    }

    /// M-A (`docs/lossless_auto_reload_impl_work_order.md` §1.1): `apply_mode`
    /// is additive + serde-defaulted. An OLD producer's JSON (no field) reads
    /// `Incremental` (the mixed-version fail-safe), `Baseline` round-trips, and
    /// the default is `Incremental`.
    #[test]
    fn apply_mode_defaults_incremental_and_round_trips() {
        assert_eq!(ApplyMode::default(), ApplyMode::Incremental);
        // An old fixture with no `apply_mode` ⇒ Incremental.
        let tx: DataplaneTransaction = serde_json::from_str(
            r#"{"generation":"g","inner_ops":[],"leaf_ops":[],"mapping_ops":[]}"#,
        )
        .expect("legacy fixture parses");
        assert_eq!(tx.apply_mode, ApplyMode::Incremental, "absent = Incremental");
        // Baseline serializes to the wire word and round-trips.
        let tx = DataplaneTransaction {
            apply_mode: ApplyMode::Baseline,
            ..tx
        };
        let json = serde_json::to_string(&tx).unwrap();
        assert!(json.contains("\"apply_mode\":\"baseline\""), "{json}");
        let back: DataplaneTransaction = serde_json::from_str(&json).unwrap();
        assert_eq!(back.apply_mode, ApplyMode::Baseline);
    }


    /// The payload-shape boundary is ONE shared predicate: floored for tiny
    /// worlds, world-scaled at scale, and the consumer mirror agrees with it
    /// (the producer's choice and the consumer's patch/rebuild agree by
    /// construction).
    #[test]
    fn shape_boundary_is_floored_and_world_scaled() {
        // The FLOOR: a tiny world still admits the sequence up to 8 ops.
        assert_eq!(bulk_shape_threshold_ops(1), SEQUENCE_FLOOR_OPS);
        assert!(!churn_is_full_world(1, SEQUENCE_FLOOR_OPS));
        assert!(churn_is_full_world(1, SEQUENCE_FLOOR_OPS + 1));
        // The CROSSOVER takes over at the 50k design point: a 9-op churn — which
        // the retired fixed 8 made a full world — is SEQUENCED there.
        assert_eq!(
            bulk_shape_threshold_ops(50_000),
            DELTA_APPLY_COW_CROSSOVER_OPS
        );
        assert!(!churn_is_full_world(50_000, 9));
        assert!(churn_is_full_world(50_000, DELTA_APPLY_COW_CROSSOVER_OPS + 1));
        // WORLD SCALING: the same churn is a full world in a small world and a
        // sequence in a big one.
        let ops = bulk_shape_threshold_ops(10_000) + 1;
        assert!(churn_is_full_world(10_000, ops));
        assert!(!churn_is_full_world(200_000, ops));
        // The consumer mirror agrees with the boundary.
        assert_eq!(DeltaApplyPlan::for_churn(10_000, ops), DeltaApplyPlan::Rebuild);
        assert_eq!(DeltaApplyPlan::for_churn(200_000, ops), DeltaApplyPlan::Cow);
    }

    /// Wire-safety: an OLD producer's mapping ops (no tunnel variants) still
    /// parse — the version gate, not serde, is the compatibility boundary.
    #[test]
    fn legacy_mapping_ops_still_parse() {
        let json = r#"[
            {"op":"map","prefix_len":24,"ip":"10.0.0.0","circuit_id":"c1","device_id":"d1"},
            {"op":"del","prefix_len":24,"ip":"10.0.0.0"},
            {"op":"clear"}
        ]"#;
        let ops: Vec<DataplaneMappingOp> = serde_json::from_str(json).unwrap();
        assert_eq!(ops.len(), 3);
    }
}

#[cfg(test)]
mod content_key_tests {
    use super::*;

    fn leaf(circuit: &str, commit: u32) -> DataplaneLeafOp {
        DataplaneLeafOp::Upsert {
            circuit_id: circuit.to_string(),
            parent_node_id: "site-1".to_string(),
            download: DataplaneRatePair {
                commit_kbps: commit,
                ceil_kbps: commit * 2,
            },
            upload: DataplaneRatePair {
                commit_kbps: commit,
                ceil_kbps: commit * 2,
            },
            aqm: DataplaneLeafAqm::None,
            admin_up: true,
        }
    }

    fn tx(generation: &str, leaves: Vec<DataplaneLeafOp>) -> DataplaneTransaction {
        DataplaneTransaction {
            generation: generation.to_string(),
            inner_ops: Vec::new(),
            leaf_ops: leaves,
            mapping_ops: Vec::new(),
            lpm_delta: Vec::new(),
            steering_vlans: Vec::new(),
            schema_version: Some(2),
            apply_mode: Default::default(),
            circuit_capacity: None,
        }
    }

    /// Op ORDER is not content: the key sorts the per-op pieces, so a reordered
    /// payload has the SAME key (a semantic no-op must not move the key).
    #[test]
    fn content_key_is_order_independent() {
        let a = tx("g", vec![leaf("c1", 10_000), leaf("c2", 20_000)]);
        let b = tx("g-other", vec![leaf("c2", 20_000), leaf("c1", 10_000)]);
        assert_eq!(content_key(&a), content_key(&b));
    }

    /// A rate edit IS content: the key moves when a rate changes.
    #[test]
    fn content_key_is_rate_sensitive() {
        let a = tx("g", vec![leaf("c1", 10_000)]);
        let b = tx("g", vec![leaf("c1", 11_000)]);
        assert_ne!(content_key(&a), content_key(&b));
    }

    /// The generation string is NOT content (it is a visibility label): two
    /// transactions with the same ops but different generations share the key.
    #[test]
    fn content_key_ignores_the_generation_label() {
        let a = tx("gen-a", vec![leaf("c1", 10_000)]);
        let b = tx("gen-b", vec![leaf("c1", 10_000)]);
        assert_eq!(content_key(&a), content_key(&b));
    }

    /// The CROSS-END CONTRACT PIN (WP3): the exact digest for a fixed payload.
    /// The broker's `translate::content_key` delegates here and the bridge
    /// stamps this into the applied-generation report — both ends MUST produce
    /// this value byte-for-byte. A regression to the algorithm FAILS here.
    #[test]
    fn content_key_matches_the_pinned_cross_end_vector() {
        let tx = tx("pinned", vec![leaf("c1", 10_000)]);
        let key = content_key(&tx);
        assert_eq!(
            key,
            "tx-4f0310c97bbee8e4ae029a01065664e9856cc88a4553db6a1bdf70396d2c70a4"
        );
    }
}

// ── the applied-world FOLD (the pure equivalence core, moved from bridge lkg.rs) ──
// ORACLE #1 (docs/seam_family3_refined_work_order.md §4a): applying a coherent
// compiled DELTA onto an applied base == applying the per-op SEQUENCE. This is
// the ONE home for the fold so both ends (and the shared-crate test) agree.
/// TAIL-only remove (bundle add appends the next id, remove pops only
/// the tail — placement.rs S1 builders), so reused slots always equal
/// the dense fold position. A future mid-table-removal mechanism
/// SILENTLY breaks this invariant — `validate_runtime_artifacts`
/// checks slot BOUNDS only, never slot→leaf identity.
#[derive(Debug, Default)]
pub struct AppliedFold {
    pub inner_ops: Vec<DataplaneInnerOp>,
    pub leaf_ops: Vec<DataplaneLeafOp>,
    pub mapping_ops: Vec<DataplaneMappingOp>,
    schema_version: Option<u32>,
    /// Op-position indexes (key → slot in the Vec) so folds stay
    /// O(ops) even when cadence rebuilds re-upsert the whole tree.
    inner_pos: std::collections::HashMap<String, usize>,
    leaf_pos: std::collections::HashMap<String, usize>,
    /// D1 per-identity mapping model: `prefix_len+ip` → op-slot in
    /// `mapping_ops`. Mappings are a SET keyed by IDENTITY, not by circuit:
    /// a circuit may hold many identities (multi-device subscribers), each
    /// distinct identity has exactly one slot, and re-binding an identity
    /// is last-writer-wins on that slot. A re-bind of a circuit's address
    /// is expressed by the producer as an explicit `Del(old)` + `Map(new)`
    /// (the rate-only path's convention), so no per-circuit churn rule is
    /// needed. Predecessor deleted by D1: the R2 `mapping_by_circuit`
    /// churn retire (which collapsed a circuit to one prefix).
    mapping_pos: std::collections::HashMap<(u8, std::net::IpAddr), usize>,
    /// T-arc: `(VNI, VLAN)` → op-slot in `mapping_ops`, the tunnel twin of
    /// `mapping_pos`. Tunnel ops share the ONE `mapping_ops` Vec so the
    /// folded op ORDER (and `Clear` semantics) match the applied stream.
    tunnel_pos: std::collections::HashMap<(u32, u16), usize>,
    /// Phase C S2: `service VLAN → op-slot` (the vlan twin of `mapping_pos`).
    vlan_pos: std::collections::HashMap<u16, usize>,
    /// Phase C S2: `MPLS label → op-slot` (the mpls twin of `mapping_pos`).
    mpls_pos: std::collections::HashMap<u32, usize>,
}

impl AppliedFold {
    /// Boot seed: the folded state starts as the boot transaction
    /// (idempotent at LKG boot — the record already holds folded
    /// content; at bundle boot it re-expresses the file tx).
    pub fn from_tx(tx: &DataplaneTransaction) -> Self {
        let mut fold = Self::default();
        fold.fold(tx);
        fold
    }

    /// §lqos-74t.30 read-only accessor: the LIVE applied parent of an inner
    /// node in the folded (last-applied) world, or `None` if absent. The
    /// bridge reads this (NOT the `AppliedState` mirror, which may be stale)
    /// as the `desired_parent == LIVE applied parent` staleness proof.
    pub fn inner_parent(&self, node_id: &str) -> Option<&str> {
        self.inner_ops.iter().find_map(|op| match op {
            DataplaneInnerOp::Upsert {
                node_id: n,
                parent_node_id,
                ..
            } if n == node_id => Some(parent_node_id.as_str()),
            _ => None,
        })
    }

    /// Folds ONE published transaction. Mapping `Clear` wipes the
    /// accumulated map ops (the tree-mirror semantics); inner/leaf
    /// `Move` is never emitted on the accepted live path and is
    /// ignored exactly like `AppliedState::extend_tx`.
    pub fn fold(&mut self, tx: &DataplaneTransaction) {
        for op in &tx.inner_ops {
            match op {
                DataplaneInnerOp::Upsert { node_id, .. } => {
                    let slot = *self
                        .inner_pos
                        .entry(node_id.clone())
                        .or_insert(self.inner_ops.len());
                    if slot == self.inner_ops.len() {
                        self.inner_ops.push(op.clone());
                    } else {
                        self.inner_ops[slot] = op.clone();
                    }
                }
                DataplaneInnerOp::Remove { node_id } => {
                    if let Some(slot) = self.inner_pos.remove(node_id) {
                        self.inner_ops.remove(slot);
                        self.reindex_inner(slot);
                    }
                }
                DataplaneInnerOp::Move { .. } => {}
            }
        }
        for op in &tx.leaf_ops {
            match op {
                DataplaneLeafOp::Upsert { circuit_id, .. } => {
                    let slot = *self
                        .leaf_pos
                        .entry(circuit_id.clone())
                        .or_insert(self.leaf_ops.len());
                    if slot == self.leaf_ops.len() {
                        self.leaf_ops.push(op.clone());
                    } else {
                        self.leaf_ops[slot] = op.clone();
                    }
                }
                DataplaneLeafOp::Remove { circuit_id } => {
                    if let Some(slot) = self.leaf_pos.remove(circuit_id) {
                        self.leaf_ops.remove(slot);
                        self.reindex_leaf(slot);
                    }
                }
                DataplaneLeafOp::Move { .. } => {}
            }
        }
        for op in &tx.mapping_ops {
            match op {
                DataplaneMappingOp::Map { prefix_len, ip, .. } => {
                    // D1 per-identity: a `Map` sets its identity slot;
                    // distinct identities never retire each other (a
                    // circuit may hold many), and the same identity
                    // re-bound is last-writer-wins on the entry.
                    let key = (*prefix_len, *ip);
                    let slot = *self
                        .mapping_pos
                        .entry(key)
                        .or_insert(self.mapping_ops.len());
                    if slot == self.mapping_ops.len() {
                        self.mapping_ops.push(op.clone());
                    } else {
                        self.mapping_ops[slot] = op.clone();
                    }
                }
                DataplaneMappingOp::Del { prefix_len, ip } => {
                    let key = (*prefix_len, *ip);
                    if let Some(slot) = self.mapping_pos.remove(&key) {
                        self.mapping_ops.remove(slot);
                        self.reindex_mapping(slot);
                    }
                }
                DataplaneMappingOp::MapTunnel { vni, vlan, .. } => {
                    let key = (*vni, *vlan);
                    let slot = *self
                        .tunnel_pos
                        .entry(key)
                        .or_insert(self.mapping_ops.len());
                    if slot == self.mapping_ops.len() {
                        self.mapping_ops.push(op.clone());
                    } else {
                        self.mapping_ops[slot] = op.clone();
                    }
                }
                DataplaneMappingOp::DelTunnel { vni, vlan } => {
                    let key = (*vni, *vlan);
                    if let Some(slot) = self.tunnel_pos.remove(&key) {
                        self.mapping_ops.remove(slot);
                        self.reindex_mapping(slot);
                    }
                }
                DataplaneMappingOp::MapVlan { vlan, .. } => {
                    let key = *vlan;
                    let slot = *self.vlan_pos.entry(key).or_insert(self.mapping_ops.len());
                    if slot == self.mapping_ops.len() {
                        self.mapping_ops.push(op.clone());
                    } else {
                        self.mapping_ops[slot] = op.clone();
                    }
                }
                DataplaneMappingOp::DelVlan { vlan } => {
                    if let Some(slot) = self.vlan_pos.remove(vlan) {
                        self.mapping_ops.remove(slot);
                        self.reindex_mapping(slot);
                    }
                }
                DataplaneMappingOp::MapMpls { label, .. } => {
                    let key = *label;
                    let slot = *self.mpls_pos.entry(key).or_insert(self.mapping_ops.len());
                    if slot == self.mapping_ops.len() {
                        self.mapping_ops.push(op.clone());
                    } else {
                        self.mapping_ops[slot] = op.clone();
                    }
                }
                DataplaneMappingOp::DelMpls { label } => {
                    if let Some(slot) = self.mpls_pos.remove(label) {
                        self.mapping_ops.remove(slot);
                        self.reindex_mapping(slot);
                    }
                }
                DataplaneMappingOp::Clear => {
                    self.mapping_ops.clear();
                    self.mapping_pos.clear();
                    self.tunnel_pos.clear();
                    self.vlan_pos.clear();
                    self.mpls_pos.clear();
                }
            }
        }
        if tx.schema_version.is_some() {
            self.schema_version = tx.schema_version;
        }
    }

    fn reindex_inner(&mut self, from: usize) {
        for slot in self.inner_pos.values_mut() {
            if *slot > from {
                *slot -= 1;
            }
        }
    }

    fn reindex_leaf(&mut self, from: usize) {
        for slot in self.leaf_pos.values_mut() {
            if *slot > from {
                *slot -= 1;
            }
        }
    }

    fn reindex_mapping(&mut self, from: usize) {
        for slot in self.mapping_pos.values_mut() {
            if *slot > from {
                *slot -= 1;
            }
        }
        for slot in self.tunnel_pos.values_mut() {
            if *slot > from {
                *slot -= 1;
            }
        }
        for slot in self.vlan_pos.values_mut() {
            if *slot > from {
                *slot -= 1;
            }
        }
        for slot in self.mpls_pos.values_mut() {
            if *slot > from {
                *slot -= 1;
            }
        }
    }

    /// The bootable transaction: the folded ops in growth order under
    /// the LAST acknowledged generation (the record's generation).
    pub fn to_transaction(&self, generation: &str) -> DataplaneTransaction {
        DataplaneTransaction {
            generation: generation.to_string(),
            inner_ops: self.inner_ops.clone(),
            leaf_ops: self.leaf_ops.clone(),
            mapping_ops: self.mapping_ops.clone(),
            schema_version: self.schema_version,
        
            steering_vlans: Vec::new(), lpm_delta: Vec::new(), apply_mode: Default::default(), circuit_capacity: None, }
    }

    /// The folded leaf op for a circuit (the dynamic-CAKE driver's
    /// rate/intended-AQM source; O(1) via the fold's position index).
    pub fn leaf_op(&self, name: &str) -> Option<&DataplaneLeafOp> {
        self.leaf_pos
            .get(name)
            .and_then(|slot| self.leaf_ops.get(*slot))
    }
}

/// ORACLE #1 — the Family-3 equivalence (`docs/f3b_bridge_order_independent_apply.md`
/// §1; `docs/seam_family3_refined_work_order.md` §4a). The coherent compiled
/// DELTA must fold to the SAME world as the per-op SEQUENCE it replaces.
#[cfg(test)]
mod oracle1_equivalence_tests {
    use super::*;

    fn rate(c: u32) -> DataplaneRatePair {
        DataplaneRatePair { commit_kbps: c, ceil_kbps: c }
    }
    fn inner(node: &str, parent: &str, c: u32) -> DataplaneInnerOp {
        DataplaneInnerOp::Upsert {
            node_id: node.into(),
            parent_node_id: parent.into(),
            download: rate(c),
            upload: rate(c),
        }
    }
    fn leaf(cid: &str, parent: &str, c: u32) -> DataplaneLeafOp {
        DataplaneLeafOp::Upsert {
            circuit_id: cid.into(),
            parent_node_id: parent.into(),
            download: rate(c),
            upload: rate(c),
            aqm: DataplaneLeafAqm::None,
            admin_up: true,
        }
    }
    fn tx(g: &str, inners: Vec<DataplaneInnerOp>, leaves: Vec<DataplaneLeafOp>) -> DataplaneTransaction {
        DataplaneTransaction {
            generation: g.into(),
            inner_ops: inners,
            leaf_ops: leaves,
            mapping_ops: Vec::new(),
            schema_version: None,
            steering_vlans: Vec::new(),
            lpm_delta: Vec::new(),
            apply_mode: Default::default(),
            circuit_capacity: None,
        }
    }
    fn fold_seq(base: &DataplaneTransaction, seq: &[DataplaneTransaction]) -> DataplaneTransaction {
        let mut f = AppliedFold::from_tx(base);
        for t in seq {
            f.fold(t);
        }
        f.to_transaction("oracle")
    }
    fn fold_delta(base: &DataplaneTransaction, d: &DataplaneTransaction) -> DataplaneTransaction {
        let mut f = AppliedFold::from_tx(base);
        f.fold(d);
        f.to_transaction("oracle")
    }

    /// ORACLE #1 (equivalence): the coherent DELTA == the per-op SEQUENCE —
    /// byte/structural equality of the folded op vectors, ORDER INCLUDED.
    #[test]
    fn coherent_delta_equals_the_per_op_sequence() {
        let base = tx(
            "gen-0",
            vec![inner("site-1", "root", 10_000)],
            vec![leaf("c1", "site-1", 5_000)],
        );
        // The producer's hand-sequenced style: one class per tx.
        let t1 = tx("gen-1", vec![inner("site-2", "root", 10_000)], vec![]);
        let t2 = tx("gen-2", vec![], vec![leaf("c2", "site-2", 6_000)]);
        let t3 = tx("gen-3", vec![], vec![leaf("c3", "site-2", 7_000)]);
        let w_ref = fold_seq(&base, &[t1, t2, t3]);

        // The coherent DELTA: the whole changed set in ONE tx (same op order).
        let delta = tx(
            "gen-d",
            vec![inner("site-2", "root", 10_000)],
            vec![leaf("c2", "site-2", 6_000), leaf("c3", "site-2", 7_000)],
        );
        let w_delta = fold_delta(&base, &delta);

        assert_eq!(w_delta.inner_ops, w_ref.inner_ops, "inner_ops equal (order included)");
        assert_eq!(w_delta.leaf_ops, w_ref.leaf_ops, "leaf_ops equal (order included)");
        assert_eq!(content_key(&w_delta), content_key(&w_ref), "content_key equal");
        assert_eq!(
            structural_identity(&w_delta),
            structural_identity(&w_ref),
            "structural identity equal"
        );
        // NON-VACUOUS: the fold actually changed the world, and an EMPTY delta
        // is steady (else this test passes on a no-op fold).
        assert_ne!(content_key(&w_delta), content_key(&base), "the fold applied the delta");
        assert_eq!(
            content_key(&fold_delta(&base, &tx("steady", vec![], vec![]))),
            content_key(&base),
            "an empty delta is a steady no-op"
        );
    }

    /// ORACLE #1 (order-independence, §1.2): a PERMUTATION of the delta's ops is
    /// invariant — the world's SEMANTICS (identity) are unchanged. (The fold's
    /// growth order is the D's order; the identity is order-independent.)
    #[test]
    fn the_delta_is_order_independent() {
        let base = tx(
            "gen-0",
            vec![inner("site-1", "root", 10_000)],
            vec![leaf("c1", "site-1", 5_000)],
        );
        let delta = tx(
            "gen-d",
            vec![inner("site-2", "root", 10_000)],
            vec![leaf("c2", "site-2", 6_000), leaf("c3", "site-2", 7_000)],
        );
        let a = fold_delta(&base, &delta);
        let mut perm = delta.clone();
        perm.inner_ops.reverse();
        perm.leaf_ops.reverse();
        let b = fold_delta(&base, &perm);
        assert_eq!(content_key(&a), content_key(&b), "content_key is order-independent");
        assert_eq!(
            structural_identity(&a),
            structural_identity(&b),
            "structural identity is order-independent"
        );
    }
    /// ORACLE #1 — the R-A PRECONDITION (the boundary review): the fold is
    /// ORDER-SENSITIVE on exactly the classes F3-B's `NonExpressible::R-A`
    /// excludes (a same-identity add+remove). The exclusion is NECESSARY: without
    /// it a delta and its permutation fold to different worlds; an EXPRESSIBLE
    /// delta (distinct ids) is order-independent (no over-rejection).
    #[test]
    fn the_ra_classes_are_the_order_sensitivity_boundary() {
        let base = tx(
            "g0",
            vec![inner("site-1", "root", 10_000)],
            vec![leaf("c1", "site-1", 5_000)],
        );
        // R-A: the SAME circuit upserted AND removed (the two orders differ).
        let up = leaf("c2", "site-1", 6_000);
        let rm = DataplaneLeafOp::Remove { circuit_id: "c2".into() };
        let d_ur = tx("d", vec![], vec![up.clone(), rm.clone()]);
        let d_ru = tx("d", vec![], vec![rm, up]);
        assert_ne!(
            content_key(&fold_delta(&base, &d_ur)),
            content_key(&fold_delta(&base, &d_ru)),
            "the R-A class folds order-dependently (why F3-B excludes it)"
        );
        // EXPRESSIBLE (distinct ids): order-independent.
        let d1 = tx(
            "d",
            vec![],
            vec![leaf("c2", "site-1", 6_000), leaf("c3", "site-1", 7_000)],
        );
        let mut d2 = d1.clone();
        d2.leaf_ops.reverse();
        assert_eq!(
            content_key(&fold_delta(&base, &d1)),
            content_key(&fold_delta(&base, &d2)),
            "an expressible delta is order-independent (no over-rejection)"
        );
    }
}

// ── D1 — the per-identity mapping model (`docs/f3b_mapping_model_d1_work_order.md`) ──
// Mappings are a SET keyed by IDENTITY (`prefix_len+ip`, `(vni,vlan)`,
// `vlan`, `label`). A circuit may hold many identities (multi-device
// subscribers); distinct identities never retire each other, and a re-bind
// is the producer's explicit `Del(old)` + `Map(new)`. The four
// `*_by_circuit` churn retires are DELETED (predecessor: the R2 churn rule,
// which collapsed a circuit to one prefix and manufactured the live
// `MultipleMappingsForCircuit` reject).
#[cfg(test)]
mod d1_mapping_model_tests {
    use super::*;

    fn map(ip: &str, circuit: &str) -> DataplaneMappingOp {
        DataplaneMappingOp::Map {
            prefix_len: 32,
            ip: ip.parse().unwrap(),
            circuit_id: circuit.to_string(),
            device_id: format!("dev-{circuit}"),
        }
    }

    fn del(ip: &str) -> DataplaneMappingOp {
        DataplaneMappingOp::Del {
            prefix_len: 32,
            ip: ip.parse().unwrap(),
        }
    }

    fn map_tunnel(vni: u32, vlan: u16, circuit: &str) -> DataplaneMappingOp {
        DataplaneMappingOp::MapTunnel {
            vni,
            vlan,
            circuit: circuit.to_string(),
        }
    }

    fn tx_maps(generation: &str, mapping_ops: Vec<DataplaneMappingOp>) -> DataplaneTransaction {
        DataplaneTransaction {
            generation: generation.to_string(),
            inner_ops: Vec::new(),
            leaf_ops: Vec::new(),
            mapping_ops,
            lpm_delta: Vec::new(),
            steering_vlans: Vec::new(),
            schema_version: None,
            apply_mode: Default::default(),
            circuit_capacity: None,
        }
    }

    fn fold(base: &DataplaneTransaction, d: &DataplaneTransaction) -> DataplaneTransaction {
        let mut f = AppliedFold::from_tx(base);
        f.fold(d);
        f.to_transaction("d1-test")
    }

    /// U1 (a): `{Map(A→C), Map(B→C)}` is order-independent AND complete —
    /// both identities survive (the live multi-device shape). Under the old
    /// per-circuit churn this collapsed to one entry.
    #[test]
    fn multi_map_for_one_circuit_is_order_independent_and_complete() {
        let base = tx_maps("g0", vec![]);
        let a = tx_maps("d", vec![map("192.0.2.1", "c1"), map("192.0.2.2", "c1")]);
        let mut b = a.clone();
        b.mapping_ops.reverse();
        let fa = fold(&base, &a);
        let fb = fold(&base, &b);
        assert_eq!(fa.mapping_ops.len(), 2, "both mappings survive");
        assert!(fa.mapping_ops.contains(&map("192.0.2.1", "c1")));
        assert!(fa.mapping_ops.contains(&map("192.0.2.2", "c1")));
        assert_eq!(content_key(&fa), content_key(&fb), "order-independent");
    }

    /// U1 (a) cross-family: a prefix `Map` and a `MapTunnel` for ONE circuit
    /// are order-independent; both survive.
    #[test]
    fn cross_family_mappings_for_one_circuit_are_order_independent() {
        let base = tx_maps("g0", vec![]);
        let a = tx_maps("d", vec![map("192.0.2.1", "c1"), map_tunnel(100, 64, "c1")]);
        let mut b = a.clone();
        b.mapping_ops.reverse();
        let fa = fold(&base, &a);
        let fb = fold(&base, &b);
        assert_eq!(fa.mapping_ops.len(), 2, "prefix + tunnel both survive");
        assert_eq!(content_key(&fa), content_key(&fb), "order-independent");
    }

    /// U1 (a): a re-bind (`Del(old)` + `Map(new)`) leaves NO stale identity.
    #[test]
    fn a_rebind_is_del_plus_map_and_leaves_no_stale_identity() {
        let base = tx_maps("g0", vec![map("192.0.2.1", "c1")]);
        let rebind = tx_maps("d", vec![del("192.0.2.1"), map("192.0.2.9", "c1")]);
        let f = fold(&base, &rebind);
        assert_eq!(
            f.mapping_ops,
            vec![map("192.0.2.9", "c1")],
            "exactly the new identity"
        );
    }

    /// U1 (b): the collision-hardening is not a mechanical delete. `Map
    /// key→C1` (delta 1) then `Map key→C2` (delta 2) ⇒ the fold keeps C2 on
    /// the shared identity, and C1 keeps its OTHER keys.
    #[test]
    fn same_identity_is_last_writer_wins_across_deltas() {
        let base = tx_maps("g0", vec![]);
        let d1 = tx_maps("d1", vec![map("192.0.2.1", "c1"), map("192.0.2.2", "c1")]);
        let d2 = tx_maps("d2", vec![map("192.0.2.1", "c2")]);
        let mut f = AppliedFold::from_tx(&base);
        f.fold(&d1);
        f.fold(&d2);
        let w = f.to_transaction("d1-test");
        assert!(w.mapping_ops.contains(&map("192.0.2.1", "c2")), "C2 owns the shared identity");
        assert!(!w.mapping_ops.contains(&map("192.0.2.1", "c1")), "C1 lost the rebound identity");
        assert!(w.mapping_ops.contains(&map("192.0.2.2", "c1")), "C1 keeps its other identity");
        assert_eq!(w.mapping_ops.len(), 2, "one op per identity");
    }

    /// Same identity twice in ONE tx is deterministic LWW in the fold; the
    /// F3-B `MappingIdentityCollision` reject (KEPT by D1) is the path guard.
    #[test]
    fn same_identity_collision_in_one_delta_is_last_writer_wins() {
        let base = tx_maps("g0", vec![]);
        let d = tx_maps("d", vec![map("192.0.2.1", "c1"), map("192.0.2.1", "c2")]);
        let w = fold(&base, &d);
        assert_eq!(w.mapping_ops.len(), 1, "one slot per identity");
        assert!(w.mapping_ops.contains(&map("192.0.2.1", "c2")), "last writer wins");
    }
}
