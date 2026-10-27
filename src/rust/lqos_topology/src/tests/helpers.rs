    use super::{
        DATA_PLANE_ROOT_NODE_ID, DataplaneInnerOp, DataplaneLeafAqm, DataplaneLeafOp,
        DataplaneMappingOp, DataplaneRatePair, EffectiveTopologyArtifacts,
        DirectCircuitHosts, acquire_effective_publish_lock, aqm_from_override,
        apply_effective_topology_to_canonical_state as try_apply_effective_topology_to_canonical_state,
        apply_effective_topology_to_network_json as try_apply_effective_topology_to_network_json,
        apply_effective_topology_to_network_json_from_canonical as try_apply_effective_topology_to_network_json_from_canonical,
        apply_health_to_option, build_effective_topology_artifacts,
        build_effective_topology_artifacts_from_canonical, build_shaping_inputs,
        collect_direct_circuit_node_ids, collect_direct_circuit_node_names,
        compile_dataplane_transaction, compute_effective_state, parse_probe_ip,
        publish_effective_topology_artifacts, publish_topology_runtime_error_status,
        ranked_auto_attachment_id, recompile_effective_network_commits,
        resolve_circuit_match_identity,
        validate_effective_topology_network,
    };
    use lqos_config::{
        CircuitAnchor, CircuitAnchorsFile, Config, ConfigShapedDevices, ShapedDevice,
        TopologyAllowedParent, TopologyAttachmentHealthStateFile, TopologyAttachmentHealthStatus,
        TopologyAttachmentOption, TopologyAttachmentRateSource, TopologyAttachmentRole,
        TopologyCanonicalIngressKind, TopologyCanonicalNode, TopologyCanonicalRateInput,
        TopologyCanonicalRateInputSource, TopologyCanonicalStateFile, TopologyEditorNode,
        TopologyEditorStateFile, TopologyEffectiveAttachmentState, TopologyEffectiveNodeState,
        TopologyEffectiveStateFile, TopologyRuntimeStatusFile,
        TopologyShapingInputsFile, TopologyShapingResolutionSource, TunnelCircuitEntry,
        TunnelCircuitsFile, TunnelIdentity,
        compute_effective_network_file_generation,
        topology_auto_attachment_option as auto_attachment_option, topology_effective_network_path,
        topology_effective_state_path, topology_operator_view_path, topology_runtime_status_path,
        topology_shaping_inputs_path,
    };
    use lqos_overrides::{TopologyAttachmentMode, TopologyOverridesFile};
    use serde_json::{Value, json};
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough for tests")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique}"));
        fs::create_dir_all(&path).expect("temp directory should be creatable");
        path
    }

    /// Hermetic [`Config`] for tests: the operator override layer is written
    /// under `lqos_directory`, so the default `/opt/libreqos/src` would make a
    /// fixture write a machine-local install path (ENOENT on a runner without
    /// the app installed). Point it at a unique temp dir instead.
    fn test_config() -> Config {
        let mut config = Config::default();
        config.lqos_directory = unique_temp_dir("lqos-topology-config")
            .to_string_lossy()
            .to_string();
        config
    }
