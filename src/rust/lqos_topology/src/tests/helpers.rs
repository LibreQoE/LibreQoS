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
        compile_dataplane_transaction, compile_dataplane_transaction_with_sqm,
        compute_effective_state, load_shaping_inputs_at_path, parse_probe_ip,
        publish_effective_topology_artifacts, publish_topology_runtime_error_status,
        ranked_auto_attachment_id, recompile_effective_network_commits,
        resolve_circuit_match_identity,
        validate_effective_topology_network,
        validate_effective_network_identity_and_cycles,
        same_world_ignoring_origin_marker,
        validate_effective_site_loss_from_canonical,
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

    /// Test-only `tracing` sink: records the WARN lines emitted on THIS thread.
    ///
    /// A log line is a shipped behaviour (u6 A1: one line per world, never per
    /// publish), so the gate has to read the log, not a mirror of the code that
    /// emits it. `lqos_topology` deliberately carries no `tracing-subscriber`
    /// dependency, so the capture is the eight `Subscriber` methods and a
    /// `message`-field visitor.
    #[derive(Clone, Default)]
    struct WarningCapture {
        lines: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl WarningCapture {
        /// Runs `body` with this capture as the thread-local default subscriber
        /// and returns the WARN lines it emitted.
        fn capturing(body: impl FnOnce()) -> Vec<String> {
            let capture = Self::default();
            {
                let _guard = tracing::subscriber::set_default(capture.clone());
                body();
            }
            capture.lines.lock().expect("capture lock").clone()
        }
    }

    impl tracing::Subscriber for WarningCapture {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() == tracing::Level::WARN
        }
        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut visitor = WarningMessage(None);
            event.record(&mut visitor);
            if let Some(message) = visitor.0 {
                self.lines.lock().expect("capture lock").push(message);
            }
        }
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }

    /// Pulls the `message` field out of one event.
    struct WarningMessage(Option<String>);

    impl tracing::field::Visit for WarningMessage {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.0 = Some(format!("{value:?}"));
            }
        }
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "message" {
                self.0 = Some(value.to_string());
            }
        }
    }
