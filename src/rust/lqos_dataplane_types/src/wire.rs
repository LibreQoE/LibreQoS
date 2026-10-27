//! Channel protocol v2 wire contract (docs/protocol_v2_card.md §1-§2, as
//! amended by the fresh-#12 closures 3/(a)).
//!
//! SINGLE HOME (unit 1a — protocol-v2 order): moved verbatim from the
//! bridge's transaction_compiler/src/wire.rs (rob-dpdk 0488ed8), which
//! had itself held it for unit 0 pending this crate's existence. This
//! module's serde contract IS the wire: the producer encodes these
//! types, the bridge parses shape-identical mirrors — never a
//! `serde_json::Value` round-trip between the mirror types.

use serde::{Deserialize, Serialize};

use crate::DataplaneTransaction;

/// The ONE protocol version (card §1 / closure 3): written into the
/// segment header at create, checked at attach, and prefixed onto every
/// frame body. No per-side hand-written version anywhere — both
/// consumers compile THIS const.
///
/// U1 (W2/W3): bumped 4 → 5 with the ack-slot growth (256 → 512 + the typed
/// reject detail). **F3-B §3b: bumped 5 → 6 with `AckOutcome::RebuildRequired`
/// (code 11) — the distinct outcome for a `NonExpressible`/owner-set delta (the
/// R3 cold-rebuild arm), so the broker no longer conflates it with a mechanics
/// `RejectedStructuralOp`.** The segment `version` == this const, so an OLD
/// lqosd against a NEW bridge (or vice versa) FAILS attach rather than walking
/// the ack ring at different strides (the silent split-brain, the implementer W2).
/// Coordinate the upgrade (W5).
pub const PROTOCOL_VERSION: u32 = 6;

/// Sibling const for the LKG persisted-envelope version (config-surface
/// card M2; protocol-v2 closure 3 disposition). Independent of the wire
/// version: an LKG from an older ENGINE is stale even when the channel
/// protocol matches.
pub const LKG_SCHEMA_VERSION: u32 = 1;

/// Batch reassembly bound (card §3): n ≤ 16 frames per batch, one
/// reassembly buffer, one in-flight batch. Exceeding is producer-local
/// (G11 TooLarge semantics); the driver defends with a typed reject.
pub const MAX_BATCH_FRAMES: u16 = 16;

// ---------------------------------------------------------------------------
// The control-channel SEGMENT + ACK-FRAME contract (U1 / W1: ONE definition).
//
// These were HAND-COPIED in the bridge's `dpdk_core/control_channel.rs` AND
// lqosd's `bridge_broker/seam.rs`. A field added to one copy only is a SILENT
// layout mismatch (both `size_of` asserts still pass) and the two sides walk
// the ring with different strides. THE SINGLE HOME is HERE: both ends compile
// THESE types — no third copy (the implementer W1).
// ---------------------------------------------------------------------------

/// Segment header magic ("LQ3C").
pub const SEGMENT_MAGIC: u32 = 0x4C51_3343;
/// Fixed segment-header size (bytes).
pub const HEADER_BYTES: usize = 320;
/// Command-ring slot size + capacity (card §2 G1-measured binding decision).
pub const CMD_SLOT_BYTES: usize = 64 * 1024 * 1024;
pub const CMD_SLOTS: usize = 4;
/// Bytes of the length prefix inside a command slot.
pub const CMD_LEN_PREFIX_BYTES: usize = 8;
/// Ack-ring slot size + capacity (fixed layout, no length prefix).
///
/// W3 (U1): GROWN from 256 → 512 to carry the typed reject DETAIL (reason +
/// op-class + identity). Both ends read THIS const (the hard-coded stride was
/// the silent-misread hazard, the implementer W2); the growth is a wire break ⇒
/// PROTOCOL_VERSION bump (below).
pub const ACK_SLOT_BYTES: usize = 512;
pub const ACK_SLOTS: usize = 8;
/// Fixed ack-frame overhead: `outcome(1)+reason(1)+op_class(1)+pad(5) +
/// fence_epoch(8) + generation_len(4) + identity_len(4)`.
pub const ACK_FRAME_FIXED: usize = 24;
/// Maximum generation string carried in an ack (UNCHANGED at 232 — the growth
/// went to the reject identity, so a long generation never evicts the detail).
/// lqosd idempotency keys stay product-side (G12).
pub const ACK_GENERATION_CAP: usize = 232;
/// Maximum reject-IDENTITY string (the node/circuit/`multi-add@site` label).
pub const ACK_IDENTITY_CAP: usize = ACK_SLOT_BYTES - ACK_FRAME_FIXED - ACK_GENERATION_CAP;

/// Typed ack outcome. The numeric VALUES are the STABLE wire contract — never
/// renumber (a producer that predates a variant reads `None` via
/// [`AckOutcome::from_code`] and the broker classifies it `UNKNOWN_OUTCOME`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AckOutcome {
    /// Publish completed; the fence-commit value is informational (G12).
    Applied = 1,
    /// Corrupted length prefix / invalid frame shape (desync class, G9).
    RejectedInvalidFrame = 2,
    /// JSON parse failure (typed reject, no mutation).
    RejectedMalformedJson = 3,
    /// Reserved "root" id used as node_id/circuit_id (parent "root" stays legal).
    RejectedReservedRoot = 4,
    /// Remove/Move op on the live path (overwrite-only, rejects StructuralOp).
    RejectedStructuralOp = 5,
    /// Driver-side LPM prepare failed for a mapping-carrying frame.
    RejectedLpmPrepare = 6,
    /// Three publish attempts aborted pre-commit (G13): NOT applied, retry is
    /// lqosd's.
    AttemptExhausted = 7,
    /// Explicit `schema_version` different from the bridge's version.
    RejectedVersionMismatch = 8,
    /// Batch-envelope defect; the whole buffer discards.
    RejectedBatchEnvelope = 9,
    /// Transaction publish while the bridge runs `queue_mode=observe`.
    RejectedObserveMode = 10,
    /// F3-B §3b (`docs/f3b_r3_runtime_rebuild_and_s3b_work_order.md`): the delta
    /// is NOT delta-expressible (a `StructurallyInvalid` class) OR it is an
    /// owner-set/span mechanics violation — the bridge takes the SAME
    /// reconciler's second arm: a cold, content-keyed REBUILD from the compiled
    /// desired. DISTINCT from `RejectedStructuralOp` (the R1 hold + surface +
    /// retry): the broker must NOT quarantine or fence this — it waits for the
    /// post-rebuild report and re-diffs. Added with PROTOCOL_VERSION 5→6.
    RebuildRequired = 11,
}

impl AckOutcome {
    /// Decode a wire code; `None` for a code this build does not know (an older
    /// or newer peer) — the broker surfaces it `UNKNOWN_OUTCOME`, never mis-maps.
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::Applied,
            2 => Self::RejectedInvalidFrame,
            3 => Self::RejectedMalformedJson,
            4 => Self::RejectedReservedRoot,
            5 => Self::RejectedStructuralOp,
            6 => Self::RejectedLpmPrepare,
            7 => Self::AttemptExhausted,
            8 => Self::RejectedVersionMismatch,
            9 => Self::RejectedBatchEnvelope,
            10 => Self::RejectedObserveMode,
            11 => Self::RebuildRequired,
            _ => return None,
        })
    }

    /// The stable wire code.
    pub fn code(self) -> u8 {
        self as u8
    }
}

/// U1/W4: the TYPED reject reason carried in the ack detail. NUMBERED + STABLE
/// (never renumber); `Other` is the catch-all for a reason a newer peer adds, so
/// an old reader NEVER mis-maps. The DRIVER vocabulary (multi-add, multi-move,
/// retire-with-circuits, span-changing) is a SUBSET — this enum covers every
/// classify/apply reject site (a non-vacuous test enumerates them).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// No typed detail (Applied / AttemptExhausted / a pre-detail producer).
    None = 0,
    /// A payload with more than one leaf ADD (>1 add is bulk-import scope).
    MultiAdd = 1,
    /// More than one site/circuit MOVE in one transaction.
    MultiMove = 2,
    /// More than one REMOVE in one transaction.
    MultiRemove = 3,
    /// More than one node/circuit RETIRE in one transaction.
    MultiRetire = 4,
    /// A transaction MIXING adds with removes/retires/moves.
    MixedOps = 5,
    /// A retire/move of a node whose applied subtree still has circuits.
    RetireWithCircuits = 6,
    /// A live move that would change the node's span (applies at the next full
    /// recompile, not live).
    SpanChanging = 7,
    /// A new site created without an accompanying circuit add.
    NewSiteNoAdd = 8,
    /// A structural op refused because the bridge's placement bundle is absent
    /// (NOT the M-A Baseline arm — see U2/R1).
    NotInPlacementMode = 9,
    /// A full-world `Baseline` emission the bridge DECLINED (arm off / over K).
    BaselineDeclined = 10,
    /// A shell-create transaction that was not the inner-only Upsert shape.
    ShellCreateNonUpsert = 11,
    /// The driver-side LPM change-set prepare failed.
    LpmPrepare = 12,
    /// JSON parse failure.
    MalformedJson = 13,
    /// A corrupted length prefix / invalid frame shape.
    InvalidFrame = 14,
    /// A reserved "root" id used as node_id/circuit_id.
    ReservedRoot = 15,
    /// An explicit schema_version mismatch.
    VersionMismatch = 16,
    /// A batch-envelope defect.
    BatchEnvelope = 17,
    /// A publish while the bridge runs `queue_mode=observe`.
    ObserveMode = 18,
    /// An unrecognized reason code (a newer peer's variant) — never a mis-map.
    Other = 255,
}

impl RejectReason {
    /// Decode a wire code; `None` for a code this build does not know.
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::None,
            1 => Self::MultiAdd,
            2 => Self::MultiMove,
            3 => Self::MultiRemove,
            4 => Self::MultiRetire,
            5 => Self::MixedOps,
            6 => Self::RetireWithCircuits,
            7 => Self::SpanChanging,
            8 => Self::NewSiteNoAdd,
            9 => Self::NotInPlacementMode,
            10 => Self::BaselineDeclined,
            11 => Self::ShellCreateNonUpsert,
            12 => Self::LpmPrepare,
            13 => Self::MalformedJson,
            14 => Self::InvalidFrame,
            15 => Self::ReservedRoot,
            16 => Self::VersionMismatch,
            17 => Self::BatchEnvelope,
            18 => Self::ObserveMode,
            255 => Self::Other,
            _ => return None,
        })
    }

    /// The stable wire code.
    pub fn code(self) -> u8 {
        self as u8
    }
}

/// U1/W3: the offending op CLASS (a coarse tag for the operator + the UI).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpClass {
    None = 0,
    InnerUpsert = 1,
    InnerRemove = 2,
    LeafUpsert = 3,
    LeafRemove = 4,
    LeafMove = 5,
    Mapping = 6,
    Other = 255,
}

impl OpClass {
    /// Decode a wire code; `None` for a code this build does not know.
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::None,
            1 => Self::InnerUpsert,
            2 => Self::InnerRemove,
            3 => Self::LeafUpsert,
            4 => Self::LeafRemove,
            5 => Self::LeafMove,
            6 => Self::Mapping,
            255 => Self::Other,
            _ => return None,
        })
    }

    /// The stable wire code.
    pub fn code(self) -> u8 {
        self as u8
    }
}

/// The fixed ack frame (the bridge is the sole ack producer). ONE definition for
/// both ends (see the module note above); layout is `#[repr(C)]` and MUST stay
/// `size_of == ACK_SLOT_BYTES`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AckFrame {
    outcome: u8,
    /// U1/W4: the typed reject reason.
    reason: u8,
    /// U1/W3: the offending op class.
    op_class: u8,
    /// `lqos-zxc.6`: the declared drop SOURCE/TIMING carried on a pause ack
    /// ([`AckDropsKind`] code) — `0` (None) for every ack with no pause
    /// residual. This repurposes the former `_r0` reserved bytes in place, so
    /// the `#[repr(C)]` prefix stays 8 B and `size_of == ACK_SLOT_BYTES`
    /// (NO wire-layout/protocol change).
    drops_kind: u8,
    /// `lqos-zxc.6`: the drop COUNT named by [`drops_kind`](Self::drops_kind).
    /// [`ACK_DROPS_UNKNOWN`] = declared-but-unmeasured — NEVER a silent `0`
    /// whenever `drops_kind != 0`.
    dropped_packets: u32,
    /// Publish-FENCE commit value on success (informational, G12); 0 on every
    /// reject/exhaustion.
    fence_epoch: u64,
    generation_len: u32,
    identity_len: u32,
    generation: [u8; ACK_GENERATION_CAP],
    /// U1/W3: the reject IDENTITY (node/circuit/`multi-add@site`).
    identity: [u8; ACK_IDENTITY_CAP],
}

const _: () = assert!(std::mem::size_of::<AckFrame>() == ACK_SLOT_BYTES);

/// `lqos-zxc.6`: the DECLARED source/timing of a pause residual carried on an
/// ack. Named explicitly so a consumer can never present an unlabeled count;
/// this is the "name the drop source/timing" contract — the pause alert fires at
/// the point of no return, so the observed residual may not yet be measurable,
/// and a predicted value MUST be declared as such (never a silent `0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AckDropsKind {
    /// No pause residual on this ack (every non-pause ack).
    None = 0,
    /// A PREDICTED residual: the ack is emitted at the point of no return and
    /// the observed drop count is not yet measurable ⇒ the count is declared
    /// predicted (a real value, or [`ACK_DROPS_UNKNOWN`] when unmeasured).
    Predicted = 1,
    /// An OBSERVED residual: a post-rebuild measurement of dropped packets.
    Observed = 2,
}

impl AckDropsKind {
    /// The wire code for this kind.
    pub fn code(self) -> u8 {
        self as u8
    }

    /// Decode a wire code, or `None` for an unknown code.
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::None),
            1 => Some(Self::Predicted),
            2 => Some(Self::Observed),
            _ => None,
        }
    }
}

/// `lqos-zxc.6`: the sentinel for a DECLARED-but-UNMEASURED drop count
/// ([`AckDropsKind::Predicted`]) — distinct from a real `0`, so a consumer that
/// sees `drops_kind != None` and this value reports "predicted, unmeasured"
/// rather than a silent zero.
pub const ACK_DROPS_UNKNOWN: u32 = u32::MAX;

impl AckFrame {
    /// Build an ack WITHOUT a typed detail (Applied / AttemptExhausted, or a
    /// producer that predates the detail) — the code-only contract.
    pub fn new(outcome: AckOutcome, generation: &str, fence_epoch: u64) -> Self {
        Self::with_detail(
            outcome,
            RejectReason::None,
            OpClass::None,
            generation,
            "",
            fence_epoch,
        )
    }

    /// Build an ack WITH the typed reject detail (U1/W3/W4): the reason, the
    /// offending op class, and the operator-facing identity.
    pub fn with_detail(
        outcome: AckOutcome,
        reason: RejectReason,
        op_class: OpClass,
        generation: &str,
        identity: &str,
        fence_epoch: u64,
    ) -> Self {
        let mut generation_bytes = [0u8; ACK_GENERATION_CAP];
        let glen = generation.len().min(ACK_GENERATION_CAP);
        generation_bytes[..glen].copy_from_slice(&generation.as_bytes()[..glen]);
        let mut identity_bytes = [0u8; ACK_IDENTITY_CAP];
        let ilen = identity.len().min(ACK_IDENTITY_CAP);
        identity_bytes[..ilen].copy_from_slice(&identity.as_bytes()[..ilen]);
        Self {
            outcome: outcome.code(),
            reason: reason.code(),
            op_class: op_class.code(),
            drops_kind: AckDropsKind::None.code(),
            dropped_packets: 0,
            fence_epoch,
            generation_len: glen as u32,
            identity_len: ilen as u32,
            generation: generation_bytes,
            identity: identity_bytes,
        }
    }

    /// The raw outcome code (the broker classifies it).
    pub fn outcome_code(&self) -> u8 {
        self.outcome
    }

    /// The decoded outcome, or `None` for an unknown code.
    pub fn outcome(&self) -> Option<AckOutcome> {
        AckOutcome::from_code(self.outcome)
    }

    /// U1/W4: the raw reason code / the decoded reason.
    pub fn reason_code(&self) -> u8 {
        self.reason
    }

    pub fn reason(&self) -> Option<RejectReason> {
        RejectReason::from_code(self.reason)
    }

    /// U1/W3: the raw op-class code / the decoded class.
    pub fn op_class_code(&self) -> u8 {
        self.op_class
    }

    pub fn op_class(&self) -> Option<OpClass> {
        OpClass::from_code(self.op_class)
    }

    pub fn fence_epoch(&self) -> u64 {
        self.fence_epoch
    }

    /// The carried generation string (truncated at [`ACK_GENERATION_CAP`]).
    pub fn generation_str(&self) -> String {
        let len = (self.generation_len as usize).min(ACK_GENERATION_CAP);
        String::from_utf8_lossy(&self.generation[..len]).into_owned()
    }

    /// U1/W3: the carried reject identity (truncated at [`ACK_IDENTITY_CAP`]).
    pub fn identity_str(&self) -> String {
        let len = (self.identity_len as usize).min(ACK_IDENTITY_CAP);
        String::from_utf8_lossy(&self.identity[..len]).into_owned()
    }

    /// `lqos-zxc.6`: attach the DECLARED drop source/timing + count (builder,
    /// so every existing `with_detail`/`new` call site keeps its `None` default).
    pub fn with_drops(mut self, kind: AckDropsKind, dropped_packets: u32) -> Self {
        self.drops_kind = kind.code();
        self.dropped_packets = dropped_packets;
        self
    }

    /// `lqos-zxc.6`: the raw drop-kind code.
    pub fn drops_kind_code(&self) -> u8 {
        self.drops_kind
    }

    /// `lqos-zxc.6`: the decoded drop source/timing, or `None` for an unknown
    /// code (`Some(AckDropsKind::None)` is the ordinary "no residual" ack).
    pub fn drops_kind(&self) -> Option<AckDropsKind> {
        AckDropsKind::from_code(self.drops_kind)
    }

    /// `lqos-zxc.6`: the named drop count ([`ACK_DROPS_UNKNOWN`] when declared
    /// but unmeasured); meaningful only when `drops_kind != None`.
    pub fn dropped_packets(&self) -> u32 {
        self.dropped_packets
    }
}

/// The wire envelope (card §2): serde-tagged, additive-only extension
/// surface. STOP is NEVER acked — silence is the reply (closure 6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FrameBody {
    /// One complete transaction in a single frame.
    Transaction {
        transaction: DataplaneTransaction,
        /// F3-B §3b (`docs/f3b_r3_runtime_rebuild_and_s3b_work_order.md`): the
        /// broker's FULL desired world content key (`content_key(&desired)`).
        /// The bridge verifies `content_key(desired_artifact) ==
        /// desired_world_content_key` before an R3 cold rebuild. Additive
        /// (`#[serde(default)]`) — an old producer omits it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        desired_world_content_key: Option<String>,
    },
    /// One segment of a chunked transaction (card §3). The payload is the
    /// CBOR serialization of the transaction, split at the slot bound;
    /// `schema_version` (the M1 field's early carrier) MUST be `Some` on
    /// idx 0 and `None` on every later idx (closure 4).
    Batch {
        n: u16,
        idx: u16,
        generation: String,
        #[serde(default)]
        schema_version: Option<u32>,
        /// F3-B §3b: carried on idx 0 only (see `Transaction`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        desired_world_content_key: Option<String>,
        payload: Vec<u8>,
    },
    /// Graceful stop: the bridge maps this to the SAME FORCE_QUIT write
    /// site as SIGINT (closure/§2 — structural signal parity); it is
    /// never acked.
    Stop,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataplaneMappingOp;
    use std::net::IpAddr;

    fn sample_tx() -> DataplaneTransaction {
        DataplaneTransaction {
            generation: "wire-test-gen".to_string(),
            inner_ops: Vec::new(),
            leaf_ops: Vec::new(),
            mapping_ops: vec![DataplaneMappingOp::Map {
                prefix_len: 32,
                ip: "100.97.0.1".parse().unwrap(),
                circuit_id: "c1".to_string(),
                device_id: "d1".to_string(),
            }],
            schema_version: Some(PROTOCOL_VERSION),
        
            steering_vlans: Vec::new(), lpm_delta: Vec::new(), apply_mode: Default::default(), circuit_capacity: None, }
    }

    /// §10.7 model constraint, now in its single home: IpAddr rides the
    /// dotted-quad STRING on the CBOR wire (probe-proven: native IpAddr
    /// does not round-trip) and re-serialization is byte-deterministic.
    #[test]
    fn ip_addr_rides_as_dotted_quad_string_and_cbor_is_deterministic() {
        let tx = sample_tx();
        let to_vec = |v: &DataplaneTransaction| {
            let mut buf = Vec::new();
            ciborium::into_writer(v, &mut buf).expect("cbor serializes");
            buf
        };
        let bytes = to_vec(&tx);
        // The dotted-quad string is IN the bytes (not the native enum map).
        let needle = b"100.97.0.1";
        assert!(
            bytes.windows(needle.len()).any(|w| w == needle),
            "CBOR must carry the dotted-quad string form"
        );
        let parsed: DataplaneTransaction = ciborium::from_reader(&bytes[..]).expect("parses");
        assert_eq!(parsed, tx, "CBOR round-trip preserves the transaction");
        assert_eq!(
            to_vec(&parsed),
            bytes,
            "CBOR re-serialization is byte-deterministic (LKG requirement, closure 7)"
        );
    }

    /// M1: the schema_version field is present, serde-defaults to None
    /// for legacy (pre-versioning) producers, and the compiler-side const
    /// is the single source the emitted field copies.
    #[test]
    fn schema_version_defaults_none_and_emitted_value_is_the_wire_const() {
        let legacy: DataplaneTransaction = serde_json::from_str(
            r#"{"generation":"g","inner_ops":[],"leaf_ops":[],"mapping_ops":[]}"#,
        )
        .expect("legacy fixture parses");
        assert_eq!(legacy.schema_version, None, "serde default = None");
        let versioned: DataplaneTransaction = serde_json::from_str(&format!(
            r#"{{"generation":"g","inner_ops":[],"leaf_ops":[],"mapping_ops":[],"schema_version":{PROTOCOL_VERSION}}}"#
        ))
        .expect("versioned fixture parses");
        assert_eq!(versioned.schema_version, Some(PROTOCOL_VERSION));
    }

    /// The envelope's serde tag shape (kind = transaction/batch/stop) is
    /// the wire contract — pin it.
    #[test]
    fn envelope_tags_are_the_wire_contract() {
        let stop = FrameBody::Stop;
        let json = serde_json::to_string(&stop).expect("serializes");
        assert_eq!(json, r#"{"kind":"stop"}"#);
        let tx = FrameBody::Transaction {
            transaction: sample_tx(),
            desired_world_content_key: None,
        };
        let json = serde_json::to_string(&tx).expect("serializes");
        assert!(json.starts_with(r#"{"kind":"transaction""#));
    }

    /// U1/W1/W4: the ack outcome numbers are the STABLE wire contract — pin
    /// every one, and pin that an unknown code decodes to `None` (never a
    /// mis-map), so a peer's newer variant cannot be silently misclassified.
    #[test]
    fn ack_outcome_codes_are_stable_and_unknown_is_none() {
        assert_eq!(AckOutcome::Applied.code(), 1);
        assert_eq!(AckOutcome::RejectedInvalidFrame.code(), 2);
        assert_eq!(AckOutcome::RejectedMalformedJson.code(), 3);
        assert_eq!(AckOutcome::RejectedReservedRoot.code(), 4);
        assert_eq!(AckOutcome::RejectedStructuralOp.code(), 5);
        assert_eq!(AckOutcome::RejectedLpmPrepare.code(), 6);
        assert_eq!(AckOutcome::AttemptExhausted.code(), 7);
        assert_eq!(AckOutcome::RejectedVersionMismatch.code(), 8);
        assert_eq!(AckOutcome::RejectedBatchEnvelope.code(), 9);
        assert_eq!(AckOutcome::RejectedObserveMode.code(), 10);
        assert_eq!(AckOutcome::RebuildRequired.code(), 11);
        // Every known code round-trips; anything else is None.
        for code in 1..=11u8 {
            let decoded = AckOutcome::from_code(code).expect("known code decodes");
            assert_eq!(decoded.code(), code);
        }
        assert_eq!(AckOutcome::from_code(0), None);
        assert_eq!(AckOutcome::from_code(12), None);
        assert_eq!(AckOutcome::from_code(255), None);
    }

    /// U1/W1: the shared frame is exactly the slot stride (the silent-misread
    /// guard) and carries the outcome + generation; a long generation truncates.
    #[test]
    fn ack_frame_is_the_slot_stride_and_carries_the_contract() {
        assert_eq!(std::mem::size_of::<AckFrame>(), ACK_SLOT_BYTES);
        let frame = AckFrame::new(AckOutcome::RejectedStructuralOp, "gen-abc", 0);
        assert_eq!(frame.outcome_code(), 5);
        assert_eq!(frame.outcome(), Some(AckOutcome::RejectedStructuralOp));
        assert_eq!(frame.fence_epoch(), 0);
        assert_eq!(frame.generation_str(), "gen-abc");
        // `new` carries NO typed detail (the code-only contract).
        assert_eq!(frame.reason(), Some(RejectReason::None));
        assert_eq!(frame.op_class(), Some(OpClass::None));
        assert_eq!(frame.identity_str(), "");
        // A generation longer than the cap truncates to the cap (never overruns).
        let long = "x".repeat(ACK_GENERATION_CAP + 50);
        let frame = AckFrame::new(AckOutcome::Applied, &long, 7);
        assert_eq!(frame.generation_str().len(), ACK_GENERATION_CAP);
        assert_eq!(frame.fence_epoch(), 7);
    }

    /// U1/W3/W4: the typed reject detail (reason + op-class + identity) rides
    /// the SAME slot and survives the round-trip; the identity truncates at its
    /// cap. This is the operator's missing input (the code-only ack cost a whole
    /// session).
    #[test]
    fn ack_frame_carries_the_typed_reject_detail() {
        assert_eq!(std::mem::size_of::<AckFrame>(), ACK_SLOT_BYTES);
        let frame = AckFrame::with_detail(
            AckOutcome::RejectedStructuralOp,
            RejectReason::MultiAdd,
            OpClass::LeafUpsert,
            "gen-jackrabbit",
            "multi-add@site-42",
            0,
        );
        assert_eq!(frame.outcome(), Some(AckOutcome::RejectedStructuralOp));
        assert_eq!(frame.reason(), Some(RejectReason::MultiAdd));
        assert_eq!(frame.op_class(), Some(OpClass::LeafUpsert));
        assert_eq!(frame.generation_str(), "gen-jackrabbit");
        assert_eq!(frame.identity_str(), "multi-add@site-42");
        // The identity truncates at its cap (never overruns the slot).
        let long = "i".repeat(ACK_IDENTITY_CAP + 50);
        let frame = AckFrame::with_detail(
            AckOutcome::RejectedStructuralOp,
            RejectReason::MultiMove,
            OpClass::InnerUpsert,
            "g",
            &long,
            0,
        );
        assert_eq!(frame.identity_str().len(), ACK_IDENTITY_CAP);
        assert_eq!(frame.reason(), Some(RejectReason::MultiMove));
    }

    /// U1/W4: the reject-reason codes are STABLE + complete (the DRIVER
    /// vocabulary + every classify/apply class + `Other`), and an unknown code
    /// decodes to `None` (never a mis-map).
    #[test]
    fn reject_reason_codes_are_stable_and_complete() {
        assert_eq!(RejectReason::None.code(), 0);
        assert_eq!(RejectReason::MultiAdd.code(), 1);
        assert_eq!(RejectReason::MultiMove.code(), 2);
        assert_eq!(RejectReason::MultiRemove.code(), 3);
        assert_eq!(RejectReason::MultiRetire.code(), 4);
        assert_eq!(RejectReason::MixedOps.code(), 5);
        assert_eq!(RejectReason::RetireWithCircuits.code(), 6);
        assert_eq!(RejectReason::SpanChanging.code(), 7);
        assert_eq!(RejectReason::NewSiteNoAdd.code(), 8);
        assert_eq!(RejectReason::NotInPlacementMode.code(), 9);
        assert_eq!(RejectReason::BaselineDeclined.code(), 10);
        assert_eq!(RejectReason::ShellCreateNonUpsert.code(), 11);
        assert_eq!(RejectReason::LpmPrepare.code(), 12);
        assert_eq!(RejectReason::MalformedJson.code(), 13);
        assert_eq!(RejectReason::InvalidFrame.code(), 14);
        assert_eq!(RejectReason::ReservedRoot.code(), 15);
        assert_eq!(RejectReason::VersionMismatch.code(), 16);
        assert_eq!(RejectReason::BatchEnvelope.code(), 17);
        assert_eq!(RejectReason::ObserveMode.code(), 18);
        assert_eq!(RejectReason::Other.code(), 255);
        for code in (0..=18u8).chain(std::iter::once(255)) {
            let reason = RejectReason::from_code(code).expect("known code decodes");
            assert_eq!(reason.code(), code);
        }
        assert_eq!(RejectReason::from_code(19), None, "a gap code is None");
        assert_eq!(RejectReason::from_code(254), None, "an unknown code is None");
    }

    #[test]
    fn zxc6_ack_drops_field_roundtrips_without_a_layout_change() {
        // A default ack carries the "no residual" contract.
        let base = AckFrame::with_detail(
            AckOutcome::RebuildRequired,
            RejectReason::Other,
            OpClass::None,
            "gen-1",
            "shard-set-change:attach/detach:n=3:c-1,c-2,c-3",
            0,
        );
        assert_eq!(base.drops_kind(), Some(AckDropsKind::None));
        assert_eq!(base.drops_kind_code(), 0);
        assert_eq!(base.dropped_packets(), 0);

        // A declared PREDICTED residual carries the UNKNOWN sentinel (NEVER a
        // silent 0) and survives the round-trip.
        let ack = base.with_drops(AckDropsKind::Predicted, ACK_DROPS_UNKNOWN);
        assert_eq!(ack.drops_kind(), Some(AckDropsKind::Predicted));
        assert_eq!(ack.drops_kind_code(), 1);
        assert_eq!(ack.dropped_packets(), ACK_DROPS_UNKNOWN);
        assert_eq!(
            ack.identity_str(),
            "shard-set-change:attach/detach:n=3:c-1,c-2,c-3"
        );

        // An OBSERVED residual names a real count.
        let observed = base.with_drops(AckDropsKind::Observed, 1234);
        assert_eq!(observed.drops_kind(), Some(AckDropsKind::Observed));
        assert_eq!(observed.dropped_packets(), 1234);

        // The former reserved bytes were repurposed IN PLACE ⇒ no wire-stride
        // (protocol) change.
        assert_eq!(std::mem::size_of::<AckFrame>(), ACK_SLOT_BYTES);
        assert_eq!(
            AckDropsKind::from_code(3),
            None,
            "an unknown kind code is None"
        );
    }
}
