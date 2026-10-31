//! Shared topology compilation layer for projecting imported integration topology
//! into operator-selectable topology modes.

#![warn(missing_docs)]

mod bundle;
mod compile_mode;
mod ingest;
mod libreqos_network;
mod validation;

pub use bundle::{
    CompiledShapingArtifacts, CompiledTopologyBundle, ImportedTopologyBundle,
    TopologyCompiledShapingFile, TopologyImportFile, materialize_manual_topology,
};
pub use compile_mode::{TopologyCompileError, TopologyCompileMode, compile_topology};
pub use ingest::{
    TopologyIngestError, TopologyWriteOrigin, TopologyWriteReceipt, TopologyWriteSource,
    TopologyWriteState, compile_and_write_imported, compile_and_write_libreqos_network_json,
    parse_compile_mode, write_compiled_topology_outputs, write_sequence,
};
pub use libreqos_network::{
    LIBREQOS_NETWORK_FILENAME, LIBREQOS_NETWORK_MAX_BYTES, LIBREQOS_NETWORK_SCHEMA_VERSION,
    LibreQosNetworkError, LibreQosNetworkFile, NETWORK_RATE_MAX_MBPS, NetworkCircuit,
    NetworkDevice, NetworkMatchBinding, NetworkMatchConfig, NetworkMatchMode, NetworkNode,
    NetworkNodeKind, NetworkRatePlan, NetworkRatePlanBps, libreqos_network_input_path,
    parse_libreqos_network, validate_libreqos_network,
};
pub use validation::validate_compiled_bundle;
