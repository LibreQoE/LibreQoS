use serde::{Deserialize, Serialize};

/// Runtime metadata filename for circuit Ethernet advisories emitted by integrations.
pub const CIRCUIT_ETHERNET_METADATA_FILENAME: &str = "circuit_ethernet_metadata.json";

/// Collection of circuit Ethernet advisories keyed by circuit identity.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CircuitEthernetMetadataFile {
    /// Per-circuit Ethernet advisory entries.
    pub circuits: Vec<CircuitEthernetMetadata>,
}

/// Describes a detected negotiated Ethernet speed and any automatic shaping cap applied to a circuit.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CircuitEthernetMetadata {
    /// Circuit identifier as emitted to `ShapedDevices.csv`.
    pub circuit_id: String,
    /// Human-readable circuit name for UI display.
    pub circuit_name: String,
    /// Device IDs considered when determining the circuit Ethernet limit.
    pub device_ids: Vec<String>,
    /// Integration/source that produced the advisory.
    pub source: String,
    /// Negotiated Ethernet speed in Mbps for the limiting device/interface.
    pub negotiated_ethernet_mbps: u64,
    /// Requested download max before any Ethernet-based cap was applied.
    pub requested_download_mbps: f32,
    /// Requested upload max before any Ethernet-based cap was applied.
    pub requested_upload_mbps: f32,
    /// Applied download max after Ethernet-based capping.
    pub applied_download_mbps: f32,
    /// Applied upload max after Ethernet-based capping.
    pub applied_upload_mbps: f32,
    /// Whether the Ethernet advisory reduced at least one shaping direction.
    pub auto_capped: bool,
    /// Device ID of the limiting device when known.
    pub limiting_device_id: Option<String>,
    /// Device name of the limiting device when known.
    pub limiting_device_name: Option<String>,
    /// Interface name that reported the limiting Ethernet speed when known.
    pub limiting_interface_name: Option<String>,
}
