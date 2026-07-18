    #[test]
    fn shaping_inputs_remap_non_selected_attachment_anchor_to_effective_attachment() {
        let lqos_directory = unique_temp_dir("lqos-topology-attachment-anchor-remap");
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
                anchor_node_id: "attachment-old".to_string(),
                anchor_node_name: Some("Old Attachment".to_string()),
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
                nodes: vec![
                    TopologyEffectiveNodeState {
                        node_id: "attachment-old".to_string(),
                        logical_parent_node_id: "site-parent".to_string(),
                        preferred_attachment_id: None,
                        effective_attachment_id: None,
                        fallback_reason: None,
                        all_attachments_suppressed: false,
                        attachments: Vec::new(),
                    },
                    TopologyEffectiveNodeState {
                        node_id: "site-child".to_string(),
                        logical_parent_node_id: "site-parent".to_string(),
                        preferred_attachment_id: Some("attachment-new".to_string()),
                        effective_attachment_id: Some("attachment-new".to_string()),
                        fallback_reason: None,
                        all_attachments_suppressed: false,
                        attachments: Vec::new(),
                    },
                ],
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: vec![
                    TopologyEditorNode {
                        node_id: "attachment-old".to_string(),
                        node_name: "Old Attachment".to_string(),
                        ..TopologyEditorNode::default()
                    },
                    TopologyEditorNode {
                        node_id: "attachment-new".to_string(),
                        node_name: "New Attachment".to_string(),
                        ..TopologyEditorNode::default()
                    },
                    TopologyEditorNode {
                        node_id: "site-child".to_string(),
                        node_name: "Child Site".to_string(),
                        allowed_parents: vec![TopologyAllowedParent {
                            parent_node_id: "site-parent".to_string(),
                            parent_node_name: "Parent Site".to_string(),
                            attachment_options: vec![
                                sample_attachment_option("attachment-old", "Old Attachment"),
                                sample_attachment_option("attachment-new", "New Attachment"),
                            ],
                            all_attachments_suppressed: false,
                            has_probe_unavailable_attachments: false,
                        }],
                        effective_attachment_name: Some("New Attachment".to_string()),
                        ..TopologyEditorNode::default()
                    },
                ],
            },
            effective_network: Some(json!({
                "Parent Site": {
                    "id": "site-parent",
                    "name": "Parent Site",
                    "children": {
                        "New Attachment": {
                            "id": "attachment-new",
                            "name": "New Attachment",
                            "children": {
                                "Child Site": {
                                    "id": "site-child",
                                    "name": "Child Site",
                                    "children": {}
                                }
                            }
                        }
                    }
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

        assert_eq!(circuit.anchor_node_id.as_deref(), Some("attachment-old"));
        assert_eq!(circuit.effective_parent_node_id, "attachment-new");
        assert_eq!(circuit.effective_parent_node_name, "New Attachment");
        assert_eq!(
            circuit.effective_attachment_id.as_deref(),
            Some("attachment-new")
        );
        assert_eq!(
            circuit.effective_attachment_name.as_deref(),
            Some("New Attachment")
        );
    }


    #[test]
    fn shaping_inputs_resolve_virtual_attachment_owner_to_exported_parent_queue() {
        let lqos_directory = unique_temp_dir("lqos-topology-virtual-owner-anchor");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Glenn Tower Client Site\",\"device-1\",\"Device 1\",\"glenn-s1.streamitnet.com\",\"attachment-node\",\"attachment-node\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "site-pop".to_string(),
                    logical_parent_node_id: "parent-switch".to_string(),
                    preferred_attachment_id: Some("attachment-node".to_string()),
                    effective_attachment_id: Some("attachment-node".to_string()),
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
                    node_id: "site-pop".to_string(),
                    node_name: "Glenn Fiber PoP".to_string(),
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "parent-switch".to_string(),
                        parent_node_name: "Parent Switch".to_string(),
                        attachment_options: vec![sample_attachment_option(
                            "attachment-node",
                            "glenn-s1.streamitnet.com",
                        )],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    effective_attachment_name: Some("glenn-s1.streamitnet.com".to_string()),
                    ..TopologyEditorNode::default()
                }],
            },
            effective_network: Some(json!({
                "Parent Switch": {
                    "id": "parent-switch",
                    "name": "Parent Switch",
                    "children": {
                        "Glenn Fiber PoP": {
                            "id": "site-pop",
                            "name": "Glenn Fiber PoP",
                            "virtual": true,
                            "active_attachment_name": "glenn-s1.streamitnet.com",
                            "children": {}
                        }
                    }
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

        assert_eq!(circuit.effective_parent_node_id, "parent-switch");
        assert_eq!(circuit.effective_parent_node_name, "Parent Switch");
        assert_eq!(
            circuit.effective_attachment_name.as_deref(),
            Some("glenn-s1.streamitnet.com")
        );
        assert_eq!(
            circuit.resolution_source,
            lqos_config::TopologyShapingResolutionSource::TopologyAnchor
        );
    }


    #[test]
    fn shaping_inputs_reject_duplicate_circuit_shape_conflicts() {
        let lqos_directory = unique_temp_dir("lqos-topology-duplicate-circuit-shape");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment,sqm\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Tower 1\",\"tower-1\",\"\",\"aa:bb:cc:dd:ee:01\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"first\",\"cake\"\n",
                "\"circuit-1\",\"Circuit 1\",\"device-2\",\"Device 2\",\"Tower 1\",\"tower-1\",\"\",\"aa:bb:cc:dd:ee:02\",\"192.0.2.11/32\",\"\",\"20\",\"30\",\"200\",\"300\",\"second\",\"fq_codel\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

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
                    "name": "Tower 1",
                    "children": {}
                }
            })),
        };

        let err = build_shaping_inputs(&config, &artifacts)
            .expect_err("duplicate circuit conflicts should fail shaping inputs");
        let message = err.to_string();

        assert!(message.contains("conflicting circuit-level fields"));
        assert!(message.contains("Download Min Mbps"));
        assert!(message.contains("Upload Min Mbps"));
        assert!(message.contains("Download Max Mbps"));
        assert!(message.contains("Upload Max Mbps"));
        assert!(message.contains("Comment"));
        assert!(message.contains("sqm"));
    }


    #[test]
    fn shaping_inputs_resolve_legacy_parent_against_exported_effective_tree() {
        let lqos_directory = unique_temp_dir("lqos-topology-legacy-parent-resolution");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Tower 1\",\"tower-1\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

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
        let circuit = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("expected circuit");

        assert_eq!(circuit.effective_parent_node_id, "tower-1");
        assert_eq!(circuit.effective_parent_node_name, "Tower 1");
        assert_eq!(
            circuit.resolution_source,
            lqos_config::TopologyShapingResolutionSource::LegacyParent
        );
    }


    #[test]
    fn shaping_inputs_resolve_legacy_parent_by_unique_effective_name_without_id() {
        let lqos_directory = unique_temp_dir("lqos-topology-legacy-parent-name-only");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Tower 1\",\"\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

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

        assert_eq!(circuit.effective_parent_node_id, "");
        assert_eq!(circuit.effective_parent_node_name, "Tower 1");
        assert_eq!(
            circuit.resolution_source,
            lqos_config::TopologyShapingResolutionSource::LegacyParent
        );
        assert!(shaping_inputs.warnings.is_empty());
    }


    #[test]
    fn shaping_inputs_skip_virtual_effective_nodes_when_resolving_physical_parent() {
        let lqos_directory = unique_temp_dir("lqos-topology-legacy-parent-virtual");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Aggregation\",\"site-agg\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "site-agg".to_string(),
                    logical_parent_node_id: "site-root".to_string(),
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
                    node_id: "site-agg".to_string(),
                    node_name: "Aggregation".to_string(),
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Core".to_string()),
                    queue_visibility_policy:
                        TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren,
                    ..TopologyEditorNode::default()
                }],
            },
            effective_network: Some(json!({
                "Core": {
                    "id": "site-root",
                    "name": "Core",
                    "children": {
                        "Aggregation": {
                            "id": "site-agg",
                            "name": "Aggregation",
                            "virtual": true,
                            "children": {}
                        }
                    }
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

        assert_eq!(circuit.effective_parent_node_id, "site-root");
        assert_eq!(circuit.effective_parent_node_name, "Core");
        assert_eq!(
            circuit.resolution_source,
            lqos_config::TopologyShapingResolutionSource::LegacyParent
        );
    }


    #[test]
    fn shaping_inputs_skip_nested_virtual_effective_nodes_when_resolving_physical_parent() {
        let lqos_directory = unique_temp_dir("lqos-topology-legacy-parent-nested-virtual");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Access\",\"site-access\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
                "\"circuit-2\",\"Circuit 2\",\"device-2\",\"Device 2\",\"Access\",\"\",\"\",\"aa:bb:cc:dd:ee:00\",\"192.0.2.11/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

        let artifacts = EffectiveTopologyArtifacts {
            effective: TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: Some(1),
                canonical_generated_unix: Some(1),
                health_generated_unix: Some(1),
                nodes: vec![
                    TopologyEffectiveNodeState {
                        node_id: "site-agg".to_string(),
                        logical_parent_node_id: "site-root".to_string(),
                        preferred_attachment_id: None,
                        effective_attachment_id: None,
                        fallback_reason: None,
                        all_attachments_suppressed: false,
                        attachments: Vec::new(),
                    },
                    TopologyEffectiveNodeState {
                        node_id: "site-access".to_string(),
                        logical_parent_node_id: "site-agg".to_string(),
                        preferred_attachment_id: None,
                        effective_attachment_id: None,
                        fallback_reason: None,
                        all_attachments_suppressed: false,
                        attachments: Vec::new(),
                    },
                ],
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: vec![
                    TopologyEditorNode {
                        node_id: "site-agg".to_string(),
                        node_name: "Aggregation".to_string(),
                        current_parent_node_id: Some("site-root".to_string()),
                        current_parent_node_name: Some("Core".to_string()),
                        queue_visibility_policy:
                            TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren,
                        ..TopologyEditorNode::default()
                    },
                    TopologyEditorNode {
                        node_id: "site-access".to_string(),
                        node_name: "Access".to_string(),
                        current_parent_node_id: Some("site-agg".to_string()),
                        current_parent_node_name: Some("Aggregation".to_string()),
                        queue_visibility_policy:
                            TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren,
                        ..TopologyEditorNode::default()
                    },
                ],
            },
            effective_network: Some(json!({
                "Core": {
                    "id": "site-root",
                    "name": "Core",
                    "children": {
                        "Aggregation": {
                            "id": "site-agg",
                            "name": "Aggregation",
                            "virtual": true,
                            "children": {
                                "Access": {
                                    "id": "site-access",
                                    "name": "Access",
                                    "virtual": true,
                                    "children": {}
                                }
                            }
                        }
                    }
                }
            })),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build")
            .expect("shaping inputs should exist");
        for circuit_id in ["circuit-1", "circuit-2"] {
            let circuit = shaping_inputs
                .circuits
                .iter()
                .find(|circuit| circuit.circuit_id == circuit_id)
                .expect("expected circuit");

            assert_eq!(circuit.effective_parent_node_id, "site-root");
            assert_eq!(circuit.effective_parent_node_name, "Core");
            assert_eq!(
                circuit.resolution_source,
                lqos_config::TopologyShapingResolutionSource::LegacyParent
            );
        }
    }


    #[test]
    fn shaping_inputs_fallback_to_generated_parents_when_anchor_does_not_resolve() {
        let lqos_directory = unique_temp_dir("lqos-topology-missing-anchor");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"Legacy Parent\",\"legacy-parent\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
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
                anchor_node_id: "missing-anchor".to_string(),
                anchor_node_name: Some("Missing Anchor".to_string()),
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
                nodes: Vec::new(),
            },
            ui_state: TopologyEditorStateFile {
                schema_version: 1,
                source: "test".to_string(),
                generated_unix: Some(1),
                ingress_identity: None,
                nodes: Vec::new(),
            },
            effective_network: Some(json!({})),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("missing anchor should no longer fail shaping input generation")
            .expect("shaping inputs should be present");
        let circuit = shaping_inputs
            .circuits
            .iter()
            .find(|circuit| circuit.circuit_id == "circuit-1")
            .expect("circuit should be present");
        assert_eq!(circuit.effective_parent_node_id, "");
        assert_eq!(circuit.effective_parent_node_name, "");
        assert_eq!(
            circuit.resolution_source,
            lqos_config::TopologyShapingResolutionSource::RuntimeFallback
        );
        assert!(
            shaping_inputs
                .warnings
                .iter()
                .any(|warning| warning.contains("missing-anchor"))
        );
        assert!(
            shaping_inputs
                .warnings
                .iter()
                .any(|warning| warning.contains("generated parent nodes"))
        );
    }


    #[test]
    fn shaping_inputs_aggregate_missing_parent_warnings() {
        let lqos_directory = unique_temp_dir("lqos-topology-missing-parent-summary");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        let mut csv = String::from(
            "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
        );
        for circuit_number in 1..=12 {
            csv.push_str(&format!(
                "\"circuit-{circuit_number}\",\"Circuit {circuit_number}\",\"device-{circuit_number}\",\"Device {circuit_number}\",\"Missing Parent\",\"missing-parent\",\"\",\"aa:bb:cc:dd:ee:{circuit_number:02x}\",\"192.0.2.{circuit_number}/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ));
        }
        fs::write(lqos_directory.join("ShapedDevices.csv"), csv)
            .expect("ShapedDevices.csv should write");

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
            effective_network: Some(json!({})),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("shaping inputs should build")
            .expect("shaping inputs should be present");
        assert_eq!(shaping_inputs.circuits.len(), 12);
        assert!(shaping_inputs.circuits.iter().all(|circuit| {
            circuit.resolution_source
                == lqos_config::TopologyShapingResolutionSource::RuntimeFallback
        }));

        let parent_warnings = shaping_inputs
            .warnings
            .iter()
            .filter(|warning| warning.contains("parent reference(s)"))
            .collect::<Vec<_>>();
        assert_eq!(parent_warnings.len(), 1);
        assert!(parent_warnings[0].contains("12 circuit(s)"));
        assert!(parent_warnings[0].contains("circuit-1"));
        assert!(parent_warnings[0].contains("circuit-5"));
        assert!(!parent_warnings[0].contains("circuit-6"));
        assert!(parent_warnings[0].contains("7 more omitted"));
        assert_eq!(
            shaping_inputs
                .warnings
                .iter()
                .filter(|warning| warning.contains("unresolved in runtime topology"))
                .count(),
            1
        );
    }


    #[test]
    fn flat_mode_assigns_explicit_generated_parent_buckets() {
        let lqos_directory = unique_temp_dir("lqos-topology-flat-summary");
        let mut config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        config.topology.compile_mode = "flat".to_string();
        config.queues.override_available_queues = Some(2);
        fs::write(
            lqos_directory.join("ShapedDevices.csv"),
            concat!(
                "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
                "\"circuit-1\",\"Circuit 1\",\"device-1\",\"Device 1\",\"\",\"\",\"\",\"aa:bb:cc:dd:ee:ff\",\"192.0.2.10/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
                "\"circuit-2\",\"Circuit 2\",\"device-2\",\"Device 2\",\"\",\"\",\"\",\"aa:bb:cc:dd:ee:00\",\"192.0.2.11/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            ),
        )
        .expect("ShapedDevices.csv should write");

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
            effective_network: Some(json!({})),
        };

        let shaping_inputs = build_shaping_inputs(&config, &artifacts)
            .expect("flat mode shaping inputs should build")
            .expect("shaping inputs should be present");
        assert!(shaping_inputs.warnings.is_empty());
        assert_eq!(shaping_inputs.circuits.len(), 2);
        assert!(shaping_inputs.circuits.iter().all(|circuit| {
            circuit
                .effective_parent_node_name
                .starts_with("Generated_PN_")
        }));
        assert!(shaping_inputs.circuits.iter().all(|circuit| {
            circuit.resolution_source == lqos_config::TopologyShapingResolutionSource::FlatBucket
        }));
    }


    #[test]
    fn flat_mode_publishes_generated_parent_nodes_into_effective_network() {
        let mut config = Config::default();
        config.topology.compile_mode = "flat".to_string();
        config.queues.override_available_queues = Some(3);

        let canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({}));
        let artifacts = build_effective_topology_artifacts_from_canonical(
            &config,
            &canonical,
            &TopologyOverridesFile::default(),
            &TopologyAttachmentHealthStateFile::default(),
        )
        .expect("flat mode effective artifacts should build");
        let effective_network = artifacts
            .effective_network
            .expect("flat mode should publish an effective network");
        let root = effective_network
            .as_object()
            .expect("effective network should be an object");
        assert_eq!(root.len(), 3);
        for index in 0..3 {
            let name = format!("Generated_PN_{}", index + 1);
            let expected_id = format!("libreqos:generated:flat:bucket:{index}");
            let node = root
                .get(&name)
                .and_then(Value::as_object)
                .expect("generated parent node should exist");
            assert_eq!(
                node.get("id").and_then(Value::as_str),
                Some(expected_id.as_str())
            );
            assert_eq!(
                node.get("name").and_then(Value::as_str),
                Some(name.as_str())
            );
        }
    }


    fn load_stale_idless_legacy_canonical_state() -> (Config, TopologyCanonicalStateFile) {
        let lqos_directory = unique_temp_dir("lqos-topology-legacy-id-heal");
        let config = Config {
            lqos_directory: lqos_directory.to_string_lossy().to_string(),
            state_directory: None,
            ..Config::default()
        };
        let mut stale_canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({
            "Globe": {
                "children": {
                    "Nested AP": {
                        "children": {},
                        "downloadBandwidthMbps": 250,
                        "type": "ap",
                        "uploadBandwidthMbps": 250
                    }
                },
                "downloadBandwidthMbps": 500,
                "type": "site",
                "uploadBandwidthMbps": 500
            },
            "PLDT": {
                "children": {},
                "downloadBandwidthMbps": 500,
                "type": "site",
                "uploadBandwidthMbps": 500
            }
        }));
        stale_canonical.compatibility_network_json["Globe"]
            .as_object_mut()
            .expect("Globe should be an object")
            .remove("id");
        stale_canonical.compatibility_network_json["Globe"]["children"]["Nested AP"]
            .as_object_mut()
            .expect("Nested AP should be an object")
            .remove("id");
        stale_canonical.compatibility_network_json["PLDT"]
            .as_object_mut()
            .expect("PLDT should be an object")
            .remove("id");
        stale_canonical
            .save(&config)
            .expect("stale canonical state should write");

        let canonical =
            TopologyCanonicalStateFile::load(&config).expect("stale canonical state should load");
        (config, canonical)
    }

    #[test]
    fn legacy_idless_sites_load_heals_compatibility_network() {
        let (_config, canonical) = load_stale_idless_legacy_canonical_state();
        let globe_id = canonical
            .nodes
            .iter()
            .find(|node| node.node_name == "Globe")
            .expect("Globe should be imported")
            .node_id
            .as_str();
        let nested_ap_id = canonical
            .nodes
            .iter()
            .find(|node| node.node_name == "Nested AP")
            .expect("Nested AP should be imported")
            .node_id
            .as_str();
        let pldt_id = canonical
            .nodes
            .iter()
            .find(|node| node.node_name == "PLDT")
            .expect("PLDT should be imported")
            .node_id
            .as_str();

        assert_eq!(
            canonical.compatibility_network_json["Globe"]["id"].as_str(),
            Some(globe_id)
        );
        assert_eq!(
            canonical.compatibility_network_json["Globe"]["children"]["Nested AP"]["id"].as_str(),
            Some(nested_ap_id)
        );
        assert_eq!(
            canonical.compatibility_network_json["PLDT"]["id"].as_str(),
            Some(pldt_id)
        );
    }


    #[test]
    fn legacy_idless_sites_publish_valid_effective_network() {
        let (config, canonical) = load_stale_idless_legacy_canonical_state();
        let globe_id = canonical
            .nodes
            .iter()
            .find(|node| node.node_name == "Globe")
            .expect("Globe should be imported")
            .node_id
            .as_str();
        let nested_ap_id = canonical
            .nodes
            .iter()
            .find(|node| node.node_name == "Nested AP")
            .expect("Nested AP should be imported")
            .node_id
            .as_str();
        let pldt_id = canonical
            .nodes
            .iter()
            .find(|node| node.node_name == "PLDT")
            .expect("PLDT should be imported")
            .node_id
            .as_str();

        let artifacts = build_effective_topology_artifacts_from_canonical(
            &config,
            &canonical,
            &TopologyOverridesFile::default(),
            &TopologyAttachmentHealthStateFile::default(),
        )
        .expect("idless legacy site topology should publish");
        let effective_network = artifacts
            .effective_network
            .expect("legacy topology should publish effective network");

        assert_eq!(effective_network["Globe"]["id"].as_str(), Some(globe_id));
        assert_eq!(
            effective_network["Globe"]["children"]["Nested AP"]["id"].as_str(),
            Some(nested_ap_id)
        );
        assert_eq!(effective_network["PLDT"]["id"].as_str(), Some(pldt_id));
    }


    #[test]
    fn runtime_squashing_collapses_backhaul_pairs_after_attachment_selection() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Relay A": {
                        "children": {
                            "Relay B": {
                                "children": {
                                    "Child Site": {
                                        "children": {
                                            "Leaf AP": {
                                                "children": {},
                                                "downloadBandwidthMbps": 200,
                                                "id": "leaf-ap",
                                                "name": "Leaf AP",
                                                "parent_site": "Child Site",
                                                "type": "AP",
                                                "uploadBandwidthMbps": 150
                                            }
                                        },
                                        "downloadBandwidthMbps": 800,
                                        "id": "child-site",
                                        "name": "Child Site",
                                        "parent_site": "Relay B",
                                        "type": "Site",
                                        "uploadBandwidthMbps": 700
                                    }
                                },
                                "downloadBandwidthMbps": 600,
                                "id": "relay-b",
                                "name": "Relay B",
                                "parent_site": "Relay A",
                                "type": "AP",
                                "uploadBandwidthMbps": 500
                            }
                        },
                        "downloadBandwidthMbps": 900,
                        "id": "relay-a",
                        "name": "Relay A",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 400
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "child-site".to_string(),
                node_name: "Child Site".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("parent-site".to_string()),
                current_parent_node_name: Some("Parent Site".to_string()),
                current_attachment_id: Some("relay-b".to_string()),
                current_attachment_name: Some("Relay B".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "parent-site".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![sample_attachment_option("relay-b", "Relay B")],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![TopologyEffectiveNodeState {
                node_id: "child-site".to_string(),
                logical_parent_node_id: "parent-site".to_string(),
                preferred_attachment_id: Some("relay-b".to_string()),
                effective_attachment_id: Some("relay-b".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![TopologyEffectiveAttachmentState {
                    attachment_id: "relay-b".to_string(),
                    health_status: TopologyAttachmentHealthStatus::Healthy,
                    health_reason: None,
                    suppressed_until_unix: None,
                    probe_enabled: false,
                    probeable: false,
                    effective_selected: true,
                }],
            }],
        };

        let squashed =
            apply_effective_topology_to_network_json(&config, &canonical, &ui_state, &effective);
        let parent_children = squashed["Parent Site"]["children"]
            .as_object()
            .expect("parent should keep children");
        assert!(parent_children.get("Relay A").is_none());
        let child_site = parent_children
            .get("Child Site")
            .and_then(|value| value.as_object())
            .expect("child site should be squashed under parent");
        assert_eq!(child_site["parent_site"].as_str(), Some("Parent Site"));
        assert_eq!(
            child_site["active_attachment_name"].as_str(),
            Some("Relay B")
        );
        assert_eq!(child_site["downloadBandwidthMbps"].as_u64(), Some(500));
        assert_eq!(child_site["uploadBandwidthMbps"].as_u64(), Some(400));
    }


    #[test]
    fn runtime_squashing_collapses_single_attachment_hops_into_site_metadata() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Backhaul Attachment": {
                        "children": {
                            "Child Site": {
                                "children": {},
                                "downloadBandwidthMbps": 940,
                                "id": "child-site",
                                "name": "Child Site",
                                "parent_site": "Backhaul Attachment",
                                "type": "Site",
                                "uploadBandwidthMbps": 940
                            }
                        },
                        "downloadBandwidthMbps": 400,
                        "id": "backhaul-attachment",
                        "name": "Backhaul Attachment",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 400
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });

        let mut single_hop_attachment =
            sample_attachment_option("backhaul-attachment", "Backhaul Attachment");
        single_hop_attachment.capacity_mbps = Some(400);
        single_hop_attachment.download_bandwidth_mbps = Some(400);
        single_hop_attachment.upload_bandwidth_mbps = Some(400);

        let squashed = apply_effective_topology_to_network_json(
            &config,
            &canonical,
            &TopologyEditorStateFile {
                schema_version: 1,
                source: "uisp/full2".to_string(),
                generated_unix: None,
                ingress_identity: None,
                nodes: vec![TopologyEditorNode {
                    node_id: "child-site".to_string(),
                    node_name: "Child Site".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("parent-site".to_string()),
                    current_parent_node_name: Some("Parent Site".to_string()),
                    current_attachment_id: Some("backhaul-attachment".to_string()),
                    current_attachment_name: Some("Backhaul Attachment".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "parent-site".to_string(),
                        parent_node_name: "Parent Site".to_string(),
                        attachment_options: vec![single_hop_attachment],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                }],
            },
            &TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: None,
                canonical_generated_unix: None,
                health_generated_unix: None,
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "child-site".to_string(),
                    logical_parent_node_id: "parent-site".to_string(),
                    preferred_attachment_id: Some("backhaul-attachment".to_string()),
                    effective_attachment_id: Some("backhaul-attachment".to_string()),
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![TopologyEffectiveAttachmentState {
                        attachment_id: "backhaul-attachment".to_string(),
                        health_status: TopologyAttachmentHealthStatus::Healthy,
                        health_reason: None,
                        suppressed_until_unix: None,
                        probe_enabled: false,
                        probeable: false,
                        effective_selected: true,
                    }],
                }],
            },
        );
        let parent_children = squashed["Parent Site"]["children"]
            .as_object()
            .expect("parent should keep children");
        assert!(parent_children.get("Backhaul Attachment").is_none());
        let child_site = parent_children
            .get("Child Site")
            .and_then(|value| value.as_object())
            .expect("child site should be squashed under parent");
        assert_eq!(child_site["parent_site"].as_str(), Some("Parent Site"));
        assert_eq!(
            child_site["active_attachment_name"].as_str(),
            Some("Backhaul Attachment")
        );
        assert_eq!(child_site["downloadBandwidthMbps"].as_u64(), Some(400));
        assert_eq!(child_site["uploadBandwidthMbps"].as_u64(), Some(400));
    }


    #[test]
    fn runtime_squashing_fails_when_single_hop_would_overwrite_child_key() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Backhaul Attachment": {
                        "children": {
                            "Child Site": {
                                "children": {},
                                "downloadBandwidthMbps": 940,
                                "id": "child-site-new",
                                "name": "Child Site",
                                "parent_site": "Backhaul Attachment",
                                "type": "Site",
                                "uploadBandwidthMbps": 940
                            }
                        },
                        "downloadBandwidthMbps": 400,
                        "id": "backhaul-attachment",
                        "name": "Backhaul Attachment",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 400
                    },
                    "Child Site": {
                        "children": {},
                        "downloadBandwidthMbps": 100,
                        "id": "child-site-existing",
                        "name": "Child Site",
                        "parent_site": "Parent Site",
                        "type": "Site",
                        "uploadBandwidthMbps": 100
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });

        let mut single_hop_attachment =
            sample_attachment_option("backhaul-attachment", "Backhaul Attachment");
        single_hop_attachment.attachment_role = TopologyAttachmentRole::PtpBackhaul;
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "child-site-new".to_string(),
                node_name: "Child Site".to_string(),
                current_parent_node_id: Some("parent-site".to_string()),
                current_parent_node_name: Some("Parent Site".to_string()),
                current_attachment_id: Some("backhaul-attachment".to_string()),
                current_attachment_name: Some("Backhaul Attachment".to_string()),
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "parent-site".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![single_hop_attachment],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                ..TopologyEditorNode::default()
            }],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![TopologyEffectiveNodeState {
                node_id: "child-site-new".to_string(),
                logical_parent_node_id: "parent-site".to_string(),
                preferred_attachment_id: Some("backhaul-attachment".to_string()),
                effective_attachment_id: Some("backhaul-attachment".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![TopologyEffectiveAttachmentState {
                    attachment_id: "backhaul-attachment".to_string(),
                    health_status: TopologyAttachmentHealthStatus::Healthy,
                    health_reason: None,
                    suppressed_until_unix: None,
                    probe_enabled: false,
                    probeable: false,
                    effective_selected: true,
                }],
            }],
        };

        let errors = try_apply_effective_topology_to_network_json(
            &config, &canonical, &ui_state, &effective,
        )
        .expect_err("runtime squashing collision should fail export");

        assert!(
            errors
                .iter()
                .any(|error| error.contains("child key already exists"))
        );
    }
