    use super::{
        EffectiveTopologyArtifacts, QueueVirtualizationContext, acquire_effective_publish_lock,
        apply_effective_topology_to_canonical_state as try_apply_effective_topology_to_canonical_state,
        apply_effective_topology_to_network_json as try_apply_effective_topology_to_network_json,
        apply_effective_topology_to_network_json_from_canonical as try_apply_effective_topology_to_network_json_from_canonical,
        apply_health_to_option, build_effective_topology_artifacts,
        build_effective_topology_artifacts_from_canonical, build_shaping_inputs,
        collect_direct_circuit_node_ids, collect_direct_circuit_node_names,
        compute_effective_state, parse_probe_ip, publish_effective_topology_artifacts,
        publish_topology_runtime_error_status, ranked_auto_attachment_id,
        validate_effective_topology_network,
    };
    use lqos_config::{
        CircuitAnchor, CircuitAnchorsFile, Config, ConfigShapedDevices, ShapedDevice,
        TopologyAllowedParent, TopologyAttachmentHealthStateFile, TopologyAttachmentHealthStatus,
        TopologyAttachmentOption, TopologyAttachmentRateSource, TopologyAttachmentRole,
        TopologyCanonicalIngressKind, TopologyCanonicalNode, TopologyCanonicalRateInput,
        TopologyCanonicalRateInputSource, TopologyCanonicalStateFile, TopologyEditorNode,
        TopologyEditorStateFile, TopologyEffectiveAttachmentState, TopologyEffectiveNodeState,
        TopologyEffectiveStateFile, TopologyQueueVisibilityPolicy, TopologyRuntimeStatusFile,
        TopologyShapingResolutionSource, topology_auto_attachment_option as auto_attachment_option,
        topology_effective_network_path, topology_effective_state_path, topology_runtime_status_path,
        topology_shaping_inputs_path,
    };
    use lqos_overrides::{TopologyAttachmentMode, TopologyOverridesFile};
    use serde_json::{Value, json};
    use std::collections::{HashMap, HashSet};
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
