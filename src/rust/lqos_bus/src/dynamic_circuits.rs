//! Bus types for dynamic circuit overlay operations.
//!
//! `DynamicAddCircuit` and `DynamicRemoveCircuit` are high-level requests used
//! to manage runtime-only dynamic circuits. These operations apply to the
//! overlay state and never mutate `ShapedDevices.csv`.
//!
//! When callers omit identity fields, the daemon allocates stable defaults
//! such as `Dynamic <u64>` for `circuit_id` and reuses that value for
//! `device_id`.

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Attachment target for a dynamic circuit overlay request.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Allocative)]
pub struct DynamicCircuitAttachmentTarget {
    /// Stable topology node identifier from `network.json` or `network.effective.json`.
    pub node_id: Option<String>,
    /// Human-readable topology node name used when a stable ID is unavailable.
    pub node_name: Option<String>,
}

/// Optional caller-supplied identity fields for a dynamic circuit.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Allocative)]
pub struct DynamicCircuitIdentitySpec {
    /// Circuit identifier override. The daemon allocates `Dynamic <n>` when omitted.
    pub circuit_id: Option<String>,
    /// Device identifier override. Defaults to `circuit_id` when omitted.
    pub device_id: Option<String>,
    /// Human-readable circuit display name.
    pub circuit_name: Option<String>,
    /// Human-readable device display name.
    pub device_name: Option<String>,
}

/// Rate inputs for a dynamic circuit overlay request.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Allocative)]
pub struct DynamicCircuitRates {
    /// Minimum guaranteed downstream rate in Mbps.
    pub download_min_mbps: f32,
    /// Minimum guaranteed upstream rate in Mbps.
    pub upload_min_mbps: f32,
    /// Maximum downstream rate in Mbps.
    pub download_max_mbps: f32,
    /// Maximum upstream rate in Mbps.
    pub upload_max_mbps: f32,
    /// Optional SQM override such as `cake` or `fq_codel`.
    pub sqm_override: Option<String>,
}

/// Request payload for adding a dynamic circuit overlay entry.
///
/// This is a high-level overlay operation and never writes to
/// `ShapedDevices.csv`. When `identity` is omitted, the daemon allocates
/// defaults such as `Dynamic <u64>`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Allocative)]
pub struct DynamicAddCircuitRequest {
    /// Attachment target for the dynamic circuit.
    pub attachment: DynamicCircuitAttachmentTarget,
    /// Optional caller-supplied identity fields.
    pub identity: Option<DynamicCircuitIdentitySpec>,
    /// Requested shaping rates.
    pub rates: DynamicCircuitRates,
    /// IPs and CIDRs owned by this dynamic circuit.
    pub ip_cidrs: Vec<String>,
    /// Optional TTL in seconds. The daemon applies the default when omitted.
    pub ttl_seconds: Option<u64>,
}

/// Request payload for removing a dynamic circuit overlay entry.
///
/// This removes only the runtime overlay entry; it never edits
/// `ShapedDevices.csv`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Allocative)]
pub struct DynamicRemoveCircuitRequest {
    /// Stable circuit identifier to remove from the dynamic overlay.
    pub circuit_id: String,
}

/// Reply payload returned after creating a dynamic circuit overlay entry.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Allocative)]
pub struct DynamicAddCircuitReply {
    /// Allocated or accepted circuit identifier.
    pub circuit_id: String,
    /// Allocated or accepted device identifier.
    pub device_id: String,
}
