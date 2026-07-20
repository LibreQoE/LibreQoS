    #[test]
    fn parse_probe_ip_rejects_invalid_cidr_suffixes() {
        assert!(parse_probe_ip("192.0.2.1").is_some());
        assert!(parse_probe_ip("192.0.2.1/32").is_some());
        assert!(parse_probe_ip("2001:db8::1/128").is_some());
        assert!(parse_probe_ip("192.0.2.1/not-a-prefix").is_none());
        assert!(parse_probe_ip("192.0.2.1/33").is_none());
        assert!(parse_probe_ip("2001:db8::1/129").is_none());
        assert!(parse_probe_ip("192.0.2.1/24/extra").is_none());
    }

    fn apply_effective_topology_to_network_json(
        config: &Config,
        canonical_network: &Value,
        ui_state: &TopologyEditorStateFile,
        effective: &TopologyEffectiveStateFile,
    ) -> Value {
        try_apply_effective_topology_to_network_json(config, canonical_network, ui_state, effective)
            .expect("effective topology export should succeed")
    }

    fn apply_effective_topology_to_network_json_from_canonical(
        config: &Config,
        canonical_network: &Value,
        canonical: &TopologyCanonicalStateFile,
        ui_state: &TopologyEditorStateFile,
        effective: &TopologyEffectiveStateFile,
        virtualization: &QueueVirtualizationContext,
    ) -> Value {
        try_apply_effective_topology_to_network_json_from_canonical(
            config,
            canonical_network,
            canonical,
            ui_state,
            effective,
            virtualization,
        )
        .expect("effective topology export should succeed")
    }

    fn apply_effective_topology_to_canonical_state(
        config: &Config,
        canonical: &TopologyCanonicalStateFile,
        ui_state: &TopologyEditorStateFile,
        effective: &TopologyEffectiveStateFile,
        virtualization: &QueueVirtualizationContext,
    ) -> Value {
        try_apply_effective_topology_to_canonical_state(
            config,
            canonical,
            ui_state,
            effective,
            virtualization,
        )
        .expect("effective topology export should succeed")
    }

    fn canonical_node_with_rate_source(
        node_id: &str,
        node_name: &str,
        download: u64,
        upload: u64,
        source: TopologyCanonicalRateInputSource,
    ) -> TopologyCanonicalNode {
        TopologyCanonicalNode {
            node_id: node_id.to_string(),
            node_name: node_name.to_string(),
            latitude: None,
            longitude: None,
            node_kind: "Site".to_string(),
            is_virtual: false,
            current_parent_node_id: None,
            current_parent_node_name: None,
            current_attachment_id: None,
            current_attachment_name: None,
            can_move: true,
            allowed_parents: Vec::new(),
            queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
            rate_input: TopologyCanonicalRateInput {
                intrinsic_download_mbps: Some(download),
                intrinsic_upload_mbps: Some(upload),
                legacy_imported_download_mbps: Some(download),
                legacy_imported_upload_mbps: Some(upload),
                source,
            },
        }
    }

    fn sample_effective_bandwidth_tree() -> serde_json::Map<String, Value> {
        json!({
            "Hoodoo Hill": {
                "children": {
                    "HoodooHill-Thunderhill": {
                        "children": {},
                        "downloadBandwidthMbps": 214,
                        "id": "ap-thunderhill",
                        "name": "HoodooHill-Thunderhill",
                        "type": "AP",
                        "uploadBandwidthMbps": 773
                    }
                },
                "downloadBandwidthMbps": 774,
                "id": "site-hoodoo",
                "name": "Hoodoo Hill",
                "type": "Site",
                "uploadBandwidthMbps": 774
            }
        })
        .as_object()
        .expect("sample tree should be an object")
        .clone()
    }


    #[test]
    fn native_compatibility_export_rates_do_not_clamp_auto_child_nodes() {
        let mut root = sample_effective_bandwidth_tree();
        let canonical = TopologyCanonicalStateFile {
            ingress_kind: TopologyCanonicalIngressKind::NativeIntegration,
            nodes: vec![
                canonical_node_with_rate_source(
                    "site-hoodoo",
                    "Hoodoo Hill",
                    774,
                    774,
                    TopologyCanonicalRateInputSource::AttachmentMax,
                ),
                canonical_node_with_rate_source(
                    "ap-thunderhill",
                    "HoodooHill-Thunderhill",
                    214,
                    773,
                    TopologyCanonicalRateInputSource::CompatibilityExport,
                ),
            ],
            ..Default::default()
        };

        super::recompile_effective_network_bandwidths(
            &mut root,
            &canonical,
            &TopologyEditorStateFile::default(),
            &TopologyEffectiveStateFile::default(),
        );

        let ap = root["Hoodoo Hill"]["children"]["HoodooHill-Thunderhill"]
            .as_object()
            .expect("AP node should exist");
        assert_eq!(
            super::node_bandwidth_mbps(ap, "downloadBandwidthMbps"),
            Some(774)
        );
        assert_eq!(
            super::node_bandwidth_mbps(ap, "uploadBandwidthMbps"),
            Some(774)
        );
    }


    #[test]
    fn legacy_imported_rates_still_cap_child_nodes() {
        let mut root = sample_effective_bandwidth_tree();
        let canonical = TopologyCanonicalStateFile {
            ingress_kind: TopologyCanonicalIngressKind::LegacyNetworkJson,
            nodes: vec![
                canonical_node_with_rate_source(
                    "site-hoodoo",
                    "Hoodoo Hill",
                    774,
                    774,
                    TopologyCanonicalRateInputSource::ImportedNetworkJson,
                ),
                canonical_node_with_rate_source(
                    "ap-thunderhill",
                    "HoodooHill-Thunderhill",
                    214,
                    773,
                    TopologyCanonicalRateInputSource::ImportedNetworkJson,
                ),
            ],
            ..Default::default()
        };

        super::recompile_effective_network_bandwidths(
            &mut root,
            &canonical,
            &TopologyEditorStateFile::default(),
            &TopologyEffectiveStateFile::default(),
        );

        let ap = root["Hoodoo Hill"]["children"]["HoodooHill-Thunderhill"]
            .as_object()
            .expect("AP node should exist");
        assert_eq!(
            super::node_bandwidth_mbps(ap, "downloadBandwidthMbps"),
            Some(214)
        );
        assert_eq!(
            super::node_bandwidth_mbps(ap, "uploadBandwidthMbps"),
            Some(773)
        );
    }


    #[test]
    fn native_uisp_compatibility_export_rates_still_cap_ap_nodes() {
        let mut root = sample_effective_bandwidth_tree();
        let canonical = TopologyCanonicalStateFile {
            source: "uisp/full".to_string(),
            ingress_kind: TopologyCanonicalIngressKind::NativeIntegration,
            nodes: vec![
                canonical_node_with_rate_source(
                    "site-hoodoo",
                    "Hoodoo Hill",
                    774,
                    774,
                    TopologyCanonicalRateInputSource::AttachmentMax,
                ),
                canonical_node_with_rate_source(
                    "ap-thunderhill",
                    "HoodooHill-Thunderhill",
                    214,
                    773,
                    TopologyCanonicalRateInputSource::CompatibilityExport,
                ),
            ],
            ..Default::default()
        };

        super::recompile_effective_network_bandwidths(
            &mut root,
            &canonical,
            &TopologyEditorStateFile::default(),
            &TopologyEffectiveStateFile::default(),
        );

        let ap = root["Hoodoo Hill"]["children"]["HoodooHill-Thunderhill"]
            .as_object()
            .expect("AP node should exist");
        assert_eq!(
            super::node_bandwidth_mbps(ap, "downloadBandwidthMbps"),
            Some(214)
        );
        assert_eq!(
            super::node_bandwidth_mbps(ap, "uploadBandwidthMbps"),
            Some(773)
        );
    }


    #[test]
    fn native_operator_override_rates_still_cap_nodes() {
        let mut root = sample_effective_bandwidth_tree();
        let canonical = TopologyCanonicalStateFile {
            ingress_kind: TopologyCanonicalIngressKind::NativeIntegration,
            nodes: vec![
                canonical_node_with_rate_source(
                    "site-hoodoo",
                    "Hoodoo Hill",
                    774,
                    774,
                    TopologyCanonicalRateInputSource::AttachmentMax,
                ),
                canonical_node_with_rate_source(
                    "ap-thunderhill",
                    "HoodooHill-Thunderhill",
                    300,
                    300,
                    TopologyCanonicalRateInputSource::OperatorOverride,
                ),
            ],
            ..Default::default()
        };

        super::recompile_effective_network_bandwidths(
            &mut root,
            &canonical,
            &TopologyEditorStateFile::default(),
            &TopologyEffectiveStateFile::default(),
        );

        let ap = root["Hoodoo Hill"]["children"]["HoodooHill-Thunderhill"]
            .as_object()
            .expect("AP node should exist");
        assert_eq!(
            super::node_bandwidth_mbps(ap, "downloadBandwidthMbps"),
            Some(300)
        );
        assert_eq!(
            super::node_bandwidth_mbps(ap, "uploadBandwidthMbps"),
            Some(300)
        );
    }

    fn write_runtime_json_fixture(path: PathBuf, value: &Value, label: &str) {
        let parent = path
            .parent()
            .expect("runtime fixture path should have parent");
        fs::create_dir_all(parent).expect("runtime fixture parent should be creatable");
        fs::write(
            &path,
            serde_json::to_string_pretty(value).expect("runtime fixture should serialize"),
        )
        .unwrap_or_else(|_| panic!("{label} should write"));
    }

    fn site_with_ap_fixture() -> (
        Config,
        TopologyCanonicalStateFile,
        TopologyEditorStateFile,
        TopologyEffectiveStateFile,
    ) {
        let mut config = Config::default();
        config.topology.queue_auto_virtualize_threshold_mbps = 5_000;
        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "splynx/ap_site".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-agg".to_string(),
                    node_name: "Aggregation".to_string(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    ..TopologyEditorNode::default()
                },
                TopologyEditorNode {
                    node_id: "ap-1".to_string(),
                    node_name: "AP One".to_string(),
                    current_parent_node_id: Some("site-agg".to_string()),
                    current_parent_node_name: Some("Aggregation".to_string()),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    ..TopologyEditorNode::default()
                },
            ],
        };
        let network = json!({
            "Aggregation": {
                "children": {
                    "AP One": {
                        "children": {},
                        "downloadBandwidthMbps": 1000,
                        "id": "ap-1",
                        "name": "AP One",
                        "type": "AP",
                        "uploadBandwidthMbps": 1000
                    }
                },
                "downloadBandwidthMbps": 7000,
                "id": "site-agg",
                "name": "Aggregation",
                "type": "Site",
                "uploadBandwidthMbps": 7000
            }
        });
        let canonical = TopologyCanonicalStateFile::from_editor_and_network(
            &editor_state,
            &network,
            TopologyCanonicalIngressKind::NativeIntegration,
        );
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![
                TopologyEffectiveNodeState {
                    node_id: "site-agg".to_string(),
                    logical_parent_node_id: String::new(),
                    ..TopologyEffectiveNodeState::default()
                },
                TopologyEffectiveNodeState {
                    node_id: "ap-1".to_string(),
                    logical_parent_node_id: "site-agg".to_string(),
                    ..TopologyEffectiveNodeState::default()
                },
            ],
        };
        (config, canonical, editor_state, effective)
    }

    fn runtime_tree_max_depth(value: &Value) -> usize {
        fn recurse(node: &Value, depth: usize) -> usize {
            let Some(map) = node.as_object() else {
                return depth;
            };
            let Some(children) = map.get("children").and_then(Value::as_object) else {
                return depth;
            };
            children
                .values()
                .map(|child| recurse(child, depth + 1))
                .max()
                .unwrap_or(depth)
        }

        value
            .as_object()
            .map(|root| {
                root.values()
                    .map(|child| recurse(child, 1))
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
    }

    fn sample_attachment_option(
        attachment_id: &str,
        attachment_name: &str,
    ) -> TopologyAttachmentOption {
        TopologyAttachmentOption {
            attachment_id: attachment_id.to_string(),
            attachment_name: attachment_name.to_string(),
            attachment_kind: "device".to_string(),
            attachment_role: TopologyAttachmentRole::PtpBackhaul,
            pair_id: None,
            peer_attachment_id: None,
            peer_attachment_name: None,
            capacity_mbps: Some(500),
            download_bandwidth_mbps: Some(500),
            upload_bandwidth_mbps: Some(500),
            transport_cap_mbps: None,
            transport_cap_reason: None,
            rate_source: TopologyAttachmentRateSource::Static,
            can_override_rate: false,
            rate_override_disabled_reason: None,
            has_rate_override: false,
            local_probe_ip: None,
            remote_probe_ip: None,
            probe_enabled: false,
            probeable: false,
            health_status: TopologyAttachmentHealthStatus::Healthy,
            health_reason: None,
            suppressed_until_unix: None,
            effective_selected: false,
        }
    }


    #[test]
    fn apply_health_to_option_marks_missing_observation_unavailable() {
        let mut option = sample_attachment_option("attachment-1", "Attachment 1");
        option.pair_id = Some("pair-1".to_string());
        option.local_probe_ip = Some("192.0.2.1".to_string());
        option.remote_probe_ip = Some("192.0.2.2".to_string());
        option.probe_enabled = true;
        option.probeable = true;

        let enriched =
            apply_health_to_option(&option, &TopologyOverridesFile::default(), &HashMap::new());

        assert_eq!(
            enriched.health_status,
            TopologyAttachmentHealthStatus::ProbeUnavailable
        );
        assert_eq!(
            enriched.health_reason.as_deref(),
            Some("Probe unavailable: no current health observation for pair 'pair-1'")
        );
    }


    #[test]
    fn ranked_auto_attachment_prefers_healthy_before_capacity() {
        let mut healthy = sample_attachment_option("healthy-link", "Healthy Link");
        healthy.capacity_mbps = Some(100);
        healthy.download_bandwidth_mbps = Some(100);
        healthy.upload_bandwidth_mbps = Some(100);
        healthy.rate_source = TopologyAttachmentRateSource::Static;
        healthy.probeable = true;
        healthy.health_status = TopologyAttachmentHealthStatus::Healthy;

        let mut unavailable = sample_attachment_option("unavailable-link", "Unavailable Link");
        unavailable.capacity_mbps = Some(10_000);
        unavailable.download_bandwidth_mbps = Some(10_000);
        unavailable.upload_bandwidth_mbps = Some(10_000);
        unavailable.rate_source = TopologyAttachmentRateSource::DynamicIntegration;
        unavailable.probeable = true;
        unavailable.health_status = TopologyAttachmentHealthStatus::ProbeUnavailable;

        let parent = TopologyAllowedParent {
            parent_node_id: "parent-1".to_string(),
            parent_node_name: "Parent 1".to_string(),
            attachment_options: vec![auto_attachment_option(), unavailable, healthy],
            all_attachments_suppressed: false,
            has_probe_unavailable_attachments: true,
        };

        assert_eq!(
            ranked_auto_attachment_id(&parent, None).as_deref(),
            Some("healthy-link")
        );
    }

    fn sample_runtime_artifacts() -> EffectiveTopologyArtifacts {
        EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "tower-1".to_string(),
                    logical_parent_node_id: "site-a".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: Vec::new(),
                }],
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: vec![TopologyEditorNode {
                    node_id: "tower-1".to_string(),
                    node_name: "Tower 1".to_string(),
                    ..TopologyEditorNode::default()
                }],
            },
            effective_network: Some(json!({
                "Tower 1": {
                    "id": "tower-1",
                    "name": "Tower 1",
                    "children": {}
                }
            })),
        }
    }


    #[test]
    fn topology_runtime_status_transitions_from_error_to_ready() {
        let lqos_directory = unique_temp_dir("lqos-topology-runtime-status-transition");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        let generation = "generation-1";

        publish_topology_runtime_error_status(&config, generation, "topology build failed")
            .expect("failed status should publish");
        let failed = TopologyRuntimeStatusFile::load(&config).expect("failed status should load");
        assert_eq!(failed.source_generation, generation);
        assert!(!failed.ready);
        assert_eq!(failed.error.as_deref(), Some("topology build failed"));

        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Tower 1\",\"tower-1\",\"tower-1\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

        publish_effective_topology_artifacts(&config, &sample_runtime_artifacts(), generation)
            .expect("ready status should publish");
        let ready = TopologyRuntimeStatusFile::load(&config).expect("ready status should load");
        assert_eq!(ready.source_generation, generation);
        assert!(ready.ready);
        assert_eq!(ready.error, None);
        assert!(!ready.shaping_generation.is_empty());
        assert_eq!(
            ready.effective_state_path,
            topology_effective_state_path(&config)
                .to_string_lossy()
                .to_string()
        );
        assert_eq!(
            ready.effective_network_path,
            topology_effective_network_path(&config)
                .to_string_lossy()
                .to_string()
        );
        assert_eq!(
            ready.shaping_inputs_path,
            topology_shaping_inputs_path(&config)
                .to_string_lossy()
                .to_string()
        );
        assert!(topology_effective_state_path(&config).exists());
        assert!(topology_effective_network_path(&config).exists());
        assert!(topology_shaping_inputs_path(&config).exists());
        assert!(topology_runtime_status_path(&config).exists());
    }

    #[test]
    fn publish_runtime_status_hashes_existing_equal_effective_network_file() {
        let lqos_directory = unique_temp_dir("lqos-topology-runtime-status-existing-network");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Tower 1\",\"tower-1\",\"tower-1\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

        let mut artifacts = sample_runtime_artifacts();
        artifacts.effective_network = Some(json!({
            "Tower 1": {
                "id": "tower-1",
                "name": "Tower 1",
                "downloadBandwidthMbps": -0.0,
                "uploadBandwidthMbps": 100.0,
                "children": {}
            }
        }));
        let effective_network_path = topology_effective_network_path(&config);
        let existing_effective_network = r#"{
  "Tower 1": {
    "id": "tower-1",
    "name": "Tower 1",
    "downloadBandwidthMbps": 0.0,
    "uploadBandwidthMbps": 100.0,
    "children": {}
  }
}"#;
        fs::create_dir_all(
            effective_network_path
                .parent()
                .expect("effective network path should have a parent"),
        )
        .expect("effective network state directory should exist");
        fs::write(
            &effective_network_path,
            existing_effective_network,
        )
        .expect("existing effective network should write");

        publish_effective_topology_artifacts(&config, &artifacts, "generation-1")
            .expect("ready status should publish");
        let ready = TopologyRuntimeStatusFile::load(&config).expect("ready status should load");
        assert!(ready.ready);
        assert_eq!(
            fs::read_to_string(&effective_network_path)
                .expect("effective network should remain readable"),
            existing_effective_network,
        );
        let effective_network_generation =
            compute_effective_network_file_generation(&effective_network_path)
                .expect("effective network generation should compute from published file");
        assert_eq!(ready.effective_generation, effective_network_generation);
    }


    #[test]
    fn publish_marks_runtime_not_ready_when_shaping_inputs_are_absent() {
        let lqos_directory = unique_temp_dir("lqos-topology-publish-no-shaping-inputs");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        let shaping_inputs_path = topology_shaping_inputs_path(&config);
        fs::create_dir_all(
            shaping_inputs_path
                .parent()
                .expect("shaping inputs path should have a parent"),
        )
        .expect("shaping state directory should exist");
        fs::write(&shaping_inputs_path, "{\"previous\":\"stale\"}\n")
            .expect("stale shaping inputs should write");

        publish_effective_topology_artifacts(&config, &sample_runtime_artifacts(), "generation-1")
            .expect("runtime artifacts should publish without shaping inputs");

        let status = TopologyRuntimeStatusFile::load(&config).expect("status should load");
        assert_eq!(status.source_generation, "generation-1");
        assert!(!status.ready);
        assert!(status.shaping_generation.is_empty());
        assert!(!status.effective_generation.is_empty());
        assert_eq!(
            status.error.as_deref(),
            Some("Topology runtime did not produce shaping inputs.")
        );
        assert!(!shaping_inputs_path.exists());
        assert!(topology_effective_state_path(&config).exists());
        assert!(topology_effective_network_path(&config).exists());
    }


    #[test]
    fn effective_publish_lock_rejects_live_holder_without_removing_lock() {
        let lqos_directory = unique_temp_dir("lqos-topology-effective-publish-lock");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            ..Config::default()
        };
        let lock_path = lqos_directory.join(super::TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_FILENAME);

        let first = acquire_effective_publish_lock(&config).expect("first lock should acquire");
        assert!(lock_path.exists());
        let second = acquire_effective_publish_lock(&config);

        assert!(second.is_err());
        assert!(lock_path.exists());
        drop(first);
        assert!(!lock_path.exists());
    }


    #[test]
    fn publish_preserves_previous_artifacts_when_shaping_inputs_fail() {
        let lqos_directory = unique_temp_dir("lqos-topology-publish-prep-failure");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        let effective_state_path = topology_effective_state_path(&config);
        let effective_network_path = topology_effective_network_path(&config);
        let shaping_inputs_path = topology_shaping_inputs_path(&config);
        let previous_effective_state = "{\"previous\":\"effective-state\"}\n";
        let previous_effective_network = "{\"previous\":\"effective-network\"}\n";
        let previous_shaping_inputs = "{\"previous\":\"shaping-inputs\"}\n";
        for path in [
            &effective_state_path,
            &effective_network_path,
            &shaping_inputs_path,
        ] {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("artifact parent directory should exist");
            }
        }
        fs::write(&effective_state_path, previous_effective_state)
            .expect("previous effective state should write");
        fs::write(&effective_network_path, previous_effective_network)
            .expect("previous effective network should write");
        fs::write(&shaping_inputs_path, previous_shaping_inputs)
            .expect("previous shaping inputs should write");
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Tower 1\",\"tower-1\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
                "\"circuit-1\",\"Circuit 1\",\"device-2\",\"Device 2\",\"Missing Parent\",\"missing-parent\",\"\",\"aa:bb:cc:dd:ee:00\",\"192.0.2.11/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

        let result = publish_effective_topology_artifacts(
            &config,
            &sample_runtime_artifacts(),
            "new-generation",
        );

        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(&effective_state_path).expect("effective state should remain"),
            previous_effective_state
        );
        assert_eq!(
            fs::read_to_string(&effective_network_path).expect("effective network should remain"),
            previous_effective_network
        );
        assert_eq!(
            fs::read_to_string(&shaping_inputs_path).expect("shaping inputs should remain"),
            previous_shaping_inputs
        );
    }


    #[test]
    fn shaping_inputs_prefer_circuit_anchors_over_csv_anchor_fields() {
        let lqos_directory = unique_temp_dir("lqos-topology-circuit-anchors");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Legacy Parent\",\"legacy-parent\",\"legacy-anchor\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");
        CircuitAnchorsFile {
            schema_version: 1,
            source: "test".to_string(),
            generated_unix: Some(1),
            anchors: vec![CircuitAnchor {
                circuit_id: "circuit-1".to_string(),
                circuit_name: Some("Circuit 1".to_string()),
                anchor_node_id: "tower-1".to_string(),
                anchor_node_name: Some("Tower 1".to_string()),
            }],
        }
        .save(&config)
        .expect("circuit_anchors.json should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "tower-1".to_string(),
                    logical_parent_node_id: "site-a".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: Vec::new(),
                }],
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: vec![TopologyEditorNode {
                    node_id: "tower-1".to_string(),
                    node_name: "Tower 1".to_string(),
                    ..TopologyEditorNode::default()
                }],
            },
            effective_network: Some(json!({
                "Tower 1": {
                    "id": "tower-1",
                    "children": {}
                }
            })),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build")
            .expect("shaping inputs should exist");
        let circuit = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("expected circuit");

        assert_eq!(circuit.anchor_node_id.as_deref(), Some("tower-1"));
        assert_eq!(circuit.anchor_node_name.as_deref(), Some("Tower 1"));
        assert_eq!(circuit.effective_parent_node_id, "tower-1");
        assert_eq!(circuit.effective_parent_node_name, "Tower 1");
    }


    #[test]
    fn shaping_inputs_apply_effective_overrides_for_integration_ingress() {
        let lqos_directory = unique_temp_dir("lqos-topology-runtime-overrides");
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        config.splynx_integration.enable_splynx = true;
        write_runtime_json_fixture(
            config.topology_state_file_path("topology_import.json"),
            &json!({
                "schema_version": 1,
                "source": "splynx/full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "compile_mode": "full",
                "imported": {
                    "source": "splynx/full",
                    "generated_unix": 1,
                    "ingress_identity": "ingress-1",
                    "compatibility_network_json": {
                        "Tower 1": {
                            "id": "tower-1",
                            "children": {}
                        }
                    },
                    "shaped_devices": [
                        {
                            "circuit_id": "circuit-1",
                            "circuit_name": "Circuit 1",
                            "device_id": "device-1",
                            "device_name": "Device 1",
                            "parent_node": "Tower 1",
                            "parent_node_id": "tower-1",
                            "anchor_node_id": null,
                            "mac": "",
                            "ipv4": [],
                            "ipv6": [],
                            "download_min_mbps": 10.0,
                            "upload_min_mbps": 10.0,
                            "download_max_mbps": 100.0,
                            "upload_max_mbps": 100.0,
                            "comment": "",
                            "sqm_override": null
                        }
                    ],
                    "circuit_anchors": {
                        "schema_version": 1,
                        "source": "splynx/full",
                        "generated_unix": 1,
                        "anchors": []
                    },
                    "ethernet_advisories": []
                }
            }),
            "topology import",
        );
        write_runtime_json_fixture(
            config.shaping_state_file_path("topology_compiled_shaping.json"),
            &json!({
                "schema_version": 1,
                "source": "splynx/full",
                "compile_mode": "full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "shaped_devices": [
                    {
                        "circuit_id": "circuit-1",
                        "circuit_name": "Circuit 1",
                        "device_id": "device-1",
                        "device_name": "Device 1",
                        "parent_node": "Tower 1",
                        "parent_node_id": "tower-1",
                        "anchor_node_id": null,
                        "mac": "",
                        "ipv4": [],
                        "ipv6": [],
                        "download_min_mbps": 10.0,
                        "upload_min_mbps": 10.0,
                        "download_max_mbps": 100.0,
                        "upload_max_mbps": 100.0,
                        "comment": "",
                        "sqm_override": null
                    }
                ],
                "circuit_anchors": {
                    "schema_version": 1,
                    "source": "splynx/full",
                    "generated_unix": 1,
                    "anchors": []
                }
            }),
            "compiled shaping",
        );
        fs::write(
            lqos_directory.join("lqos_overrides.json"),
            serde_json::to_string_pretty(&json!({
                "persistent_devices": [
                    {
                        "circuit_id": "circuit-2",
                        "circuit_name": "Circuit 2",
                        "device_id": "device-2",
                        "device_name": "Device 2",
                        "parent_node": "Tower 1",
                        "parent_node_id": "tower-1",
                        "anchor_node_id": null,
                        "mac": "",
                        "ipv4": [],
                        "ipv6": [],
                        "download_min_mbps": 5.0,
                        "upload_min_mbps": 5.0,
                        "download_max_mbps": 50.0,
                        "upload_max_mbps": 50.0,
                        "comment": "",
                        "sqm_override": null
                    }
                ],
                "circuit_adjustments": [
                    {
                        "type": "device_adjust_speed",
                        "device_id": "device-1",
                        "max_download_bandwidth": 80.0,
                        "max_upload_bandwidth": 60.0
                    }
                ],
                "network_adjustments": []
            }))
            .expect("override json should serialize"),
        )
        .expect("override file should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: Vec::new(),
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: Vec::new(),
            },
            effective_network: Some(json!({
                "Tower 1": {
                    "id": "tower-1",
                    "children": {}
                }
            })),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build")
            .expect("shaping inputs should exist");
        let circuit_one = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("expected circuit-1");
        let circuit_two = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-2")
            .expect("expected circuit-2");

        assert_eq!(circuit_one.download_max_mbps, 80.0);
        assert_eq!(circuit_one.upload_max_mbps, 60.0);
        assert_eq!(circuit_two.effective_parent_node_id, "tower-1");
        assert_eq!(circuit_two.effective_parent_node_name, "Tower 1");
    }


    #[test]
    fn shaping_inputs_reparent_override_clears_stale_csv_anchor() {
        let lqos_directory = unique_temp_dir("lqos-topology-reparent-clears-anchor");
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        config.splynx_integration.enable_splynx = true;
        let shaped_device = json!({
            "circuit_id": "circuit-1",
            "circuit_name": "Circuit 1",
            "device_id": "device-1",
            "device_name": "Device 1",
            "parent_node": "Old Tower",
            "parent_node_id": "old-tower",
            "anchor_node_id": "old-tower",
            "mac": "",
            "ipv4": [],
            "ipv6": [],
            "download_min_mbps": 10.0,
            "upload_min_mbps": 10.0,
            "download_max_mbps": 100.0,
            "upload_max_mbps": 100.0,
            "comment": "",
            "sqm_override": null
        });
        write_runtime_json_fixture(
            config.topology_state_file_path("topology_import.json"),
            &json!({
                "schema_version": 1,
                "source": "splynx/full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "compile_mode": "full",
                "imported": {
                    "source": "splynx/full",
                    "generated_unix": 1,
                    "ingress_identity": "ingress-1",
                    "compatibility_network_json": {},
                    "shaped_devices": [shaped_device.clone()],
                    "circuit_anchors": {
                        "schema_version": 1,
                        "source": "splynx/full",
                        "generated_unix": 1,
                        "anchors": []
                    },
                    "ethernet_advisories": []
                }
            }),
            "topology import",
        );
        write_runtime_json_fixture(
            config.shaping_state_file_path("topology_compiled_shaping.json"),
            &json!({
                "schema_version": 1,
                "source": "splynx/full",
                "compile_mode": "full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "shaped_devices": [shaped_device],
                "circuit_anchors": {
                    "schema_version": 1,
                    "source": "splynx/full",
                    "generated_unix": 1,
                    "anchors": []
                }
            }),
            "compiled shaping",
        );
        fs::write(
            lqos_directory.join("lqos_overrides.json"),
            serde_json::to_string_pretty(&json!({
                "circuit_adjustments": [
                    {
                        "type": "reparent_circuit",
                        "circuit_id": "circuit-1",
                        "parent_node": "New Tower"
                    }
                ],
                "network_adjustments": []
            }))
            .expect("override json should serialize"),
        )
        .expect("override file should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: Vec::new(),
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: Vec::new(),
            },
            effective_network: Some(json!({
                "Old Tower": {
                    "id": "old-tower",
                    "name": "Old Tower",
                    "children": {}
                },
                "New Tower": {
                    "id": "new-tower",
                    "name": "New Tower",
                    "children": {}
                }
            })),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build")
            .expect("shaping inputs should exist");
        let circuit = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("expected circuit");

        assert_eq!(circuit.anchor_node_id, None);
        assert_eq!(circuit.effective_parent_node_id, "new-tower");
        assert_eq!(circuit.effective_parent_node_name, "New Tower");
        assert_eq!(
            circuit.resolution_source,
            lqos_config::TopologyShapingResolutionSource::LegacyParent
        );
    }


    #[test]
    fn shaping_inputs_use_topology_import_without_shaped_devices_csv_for_integration_ingress() {
        let lqos_directory = unique_temp_dir("lqos-topology-import-shaped-devices");
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        config.uisp_integration.enable_uisp = true;
        write_runtime_json_fixture(
            config.topology_state_file_path("topology_import.json"),
            &json!({
                "schema_version": 1,
                "source": "uisp/full2",
                "compile_mode": "full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "imported": {
                    "source": "uisp/full2",
                    "generated_unix": 1,
                    "ingress_identity": "imported-1",
                    "compatibility_network_json": {},
                    "shaped_devices": [
                        {
                            "circuit_id": "circuit-1",
                            "circuit_name": "Circuit 1",
                            "device_id": "device-1",
                            "device_name": "Device 1",
                            "parent_node": "Tower 1",
                            "parent_node_id": "tower-1",
                            "anchor_node_id": null,
                            "mac": "",
                            "ipv4": [],
                            "ipv6": [],
                            "download_min_mbps": 10.0,
                            "upload_min_mbps": 10.0,
                            "download_max_mbps": 100.0,
                            "upload_max_mbps": 100.0,
                            "comment": "",
                            "sqm_override": null
                        }
                    ],
                    "circuit_anchors": {
                        "schema_version": 1,
                        "source": "uisp/full",
                        "generated_unix": 1,
                        "anchors": []
                    },
                    "ethernet_advisories": []
                }
            }),
            "topology import",
        );
        write_runtime_json_fixture(
            config.shaping_state_file_path("topology_compiled_shaping.json"),
            &json!({
                "schema_version": 1,
                "source": "uisp/full",
                "compile_mode": "full",
                "generated_unix": 1,
                "ingress_identity": "ingress-1",
                "shaped_devices": [
                    {
                        "circuit_id": "circuit-1",
                        "circuit_name": "Circuit 1",
                        "device_id": "device-1",
                        "device_name": "Device 1",
                        "parent_node": "Compiled Tower 1",
                        "parent_node_id": "compiled-tower-1",
                        "anchor_node_id": null,
                        "mac": "",
                        "ipv4": [],
                        "ipv6": [],
                        "download_min_mbps": 10.0,
                        "upload_min_mbps": 10.0,
                        "download_max_mbps": 100.0,
                        "upload_max_mbps": 100.0,
                        "comment": "",
                        "sqm_override": null
                    }
                ],
                "circuit_anchors": {
                    "schema_version": 1,
                    "source": "uisp/full",
                    "generated_unix": 1,
                    "anchors": []
                }
            }),
            "compiled shaping",
        );

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: Vec::new(),
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: Vec::new(),
            },
            effective_network: Some(json!({
                "Compiled Tower 1": {
                    "id": "compiled-tower-1",
                    "children": {}
                }
            })),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build from topology_compiled_shaping.json")
            .expect("shaping inputs should exist");
        let circuit = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("expected circuit");

        assert_eq!(circuit.effective_parent_node_id, "compiled-tower-1");
        assert_eq!(circuit.effective_parent_node_name, "Compiled Tower 1");
    }
