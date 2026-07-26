use crate::Config;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Runtime metadata filename for circuit Ethernet advisories emitted by integrations.
pub const CIRCUIT_ETHERNET_METADATA_FILENAME: &str = "circuit_ethernet_metadata.json";

/// Returns the path of the circuit Ethernet advisory runtime file.
pub fn circuit_ethernet_metadata_path(config: &Config) -> PathBuf {
    config.topology_state_read_path(CIRCUIT_ETHERNET_METADATA_FILENAME)
}

/// Collection of circuit Ethernet advisories keyed by circuit identity.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CircuitEthernetMetadataFile {
    /// Per-circuit Ethernet advisory entries.
    pub circuits: Vec<CircuitEthernetMetadata>,
}

/// Identifies the topology object whose configured rate was reduced by an Ethernet limit.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum EthernetCapTargetKind {
    /// A subscriber circuit represented by `ShapedDevices.csv`.
    #[default]
    Circuit,
    /// A topology node such as an access point or infrastructure device.
    Node,
}

/// Describes a detected negotiated Ethernet speed and any automatic rate cap applied to a circuit or topology node.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CircuitEthernetMetadata {
    /// The kind of topology object affected by the Ethernet rate cap.
    #[serde(default)]
    pub target_kind: EthernetCapTargetKind,
    /// Stable target identity. Empty legacy values fall back to `circuit_id`.
    #[serde(default)]
    pub target_id: String,
    /// Human-facing target name. Empty legacy values fall back to `circuit_name`.
    #[serde(default)]
    pub target_name: String,
    /// Circuit identifier as emitted to `ShapedDevices.csv`, retained for compatibility.
    ///
    /// For topology-node advisories, this matches `target_id`.
    pub circuit_id: String,
    /// Human-readable circuit name for UI display, retained for compatibility.
    ///
    /// For topology-node advisories, this matches `target_name`.
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

impl CircuitEthernetMetadata {
    /// Creates empty advisory metadata for a circuit or topology-node target.
    ///
    /// The legacy circuit fields mirror the target identity so existing readers can safely
    /// consume topology-node advisories.
    pub fn for_target(
        target_kind: EthernetCapTargetKind,
        target_id: String,
        target_name: String,
    ) -> Self {
        Self {
            target_kind,
            circuit_id: target_id.clone(),
            circuit_name: target_name.clone(),
            target_id,
            target_name,
            ..Default::default()
        }
    }

    /// Returns the stable identity of the capped circuit or topology node.
    pub fn target_id_or_circuit_id(&self) -> &str {
        if self.target_id.trim().is_empty() {
            &self.circuit_id
        } else {
            &self.target_id
        }
    }

    /// Returns the display name of the capped circuit or topology node.
    pub fn target_name_or_circuit_name(&self) -> &str {
        if self.target_name.trim().is_empty() {
            &self.circuit_name
        } else {
            &self.target_name
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CircuitEthernetMetadata, EthernetCapTargetKind};

    #[test]
    fn legacy_circuit_metadata_uses_circuit_identity_as_the_target() {
        let metadata = CircuitEthernetMetadata {
            circuit_id: "circuit-1".to_string(),
            circuit_name: "Circuit One".to_string(),
            ..Default::default()
        };

        assert_eq!(metadata.target_kind, EthernetCapTargetKind::Circuit);
        assert_eq!(metadata.target_id_or_circuit_id(), "circuit-1");
        assert_eq!(metadata.target_name_or_circuit_name(), "Circuit One");
    }

    #[test]
    fn target_constructor_keeps_legacy_identity_in_sync() {
        let metadata = CircuitEthernetMetadata::for_target(
            EthernetCapTargetKind::Node,
            "uisp:device:ap-1".to_string(),
            "AP One".to_string(),
        );

        assert_eq!(metadata.target_kind, EthernetCapTargetKind::Node);
        assert_eq!(metadata.circuit_id, metadata.target_id);
        assert_eq!(metadata.circuit_name, metadata.target_name);
    }
}
