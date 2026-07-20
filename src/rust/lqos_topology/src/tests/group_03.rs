    #[test]
    fn runtime_squashing_reduces_export_tree_depth_for_queue_consumers() {
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
                                        "children": {},
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

        let canonical_depth = runtime_tree_max_depth(&canonical);
        let squashed =
            apply_effective_topology_to_network_json(&config, &canonical, &ui_state, &effective);
        let squashed_depth = runtime_tree_max_depth(&squashed);

        assert_eq!(canonical_depth, 4);
        assert_eq!(squashed_depth, 2);
        assert!(squashed_depth < canonical_depth);
    }


    #[test]
    fn native_integration_effective_export_uses_logical_canonical_tree_before_squashing() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;

        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-west".to_string(),
                    node_name: "WestRedd".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "child-site".to_string(),
                    node_name: "Tuscany Ridge".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-west".to_string()),
                    current_parent_node_name: Some("WestRedd".to_string()),
                    current_attachment_id: Some("relay-b".to_string()),
                    current_attachment_name: Some("AVIAT_TuscanyRidge".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "site-west".to_string(),
                        parent_node_name: "WestRedd".to_string(),
                        attachment_options: vec![sample_attachment_option(
                            "relay-b",
                            "AVIAT_TuscanyRidge",
                        )],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let mut canonical = TopologyCanonicalStateFile::from_editor_and_network(
            &editor_state,
            &json!({
                "AVIAT_WestRedd": {
                    "children": {
                        "AVIAT_TuscanyRidge": {
                            "children": {
                                "Tuscany Ridge": {
                                    "children": {},
                                    "downloadBandwidthMbps": 900,
                                    "id": "child-site",
                                    "name": "Tuscany Ridge",
                                    "type": "Site",
                                    "uploadBandwidthMbps": 900
                                }
                            },
                            "downloadBandwidthMbps": 900,
                            "id": "relay-b",
                            "name": "AVIAT_TuscanyRidge",
                            "type": "AP",
                            "uploadBandwidthMbps": 900
                        }
                    },
                    "downloadBandwidthMbps": 1000,
                    "id": "relay-a",
                    "name": "AVIAT_WestRedd",
                    "type": "AP",
                    "uploadBandwidthMbps": 1000
                }
            }),
            TopologyCanonicalIngressKind::NativeIntegration,
        );
        canonical.nodes.push(TopologyCanonicalNode {
            node_id: "site-west".to_string(),
            node_name: "WestRedd".to_string(),
            latitude: None,
            longitude: None,
            node_kind: "Site".to_string(),
            is_virtual: false,
            current_parent_node_id: None,
            current_parent_node_name: None,
            current_attachment_id: None,
            current_attachment_name: None,
            can_move: false,
            allowed_parents: Vec::new(),
            queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
            rate_input: Default::default(),
        });

        let squashed = apply_effective_topology_to_canonical_state(
            &config,
            &canonical,
            &editor_state,
            &TopologyEffectiveStateFile {
                schema_version: 1,
                generated_unix: None,
                canonical_generated_unix: None,
                health_generated_unix: None,
                nodes: vec![TopologyEffectiveNodeState {
                    node_id: "child-site".to_string(),
                    logical_parent_node_id: "site-west".to_string(),
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
            },
            &QueueVirtualizationContext::default(),
        );

        let root_children = squashed
            .as_object()
            .expect("native effective export should be an object");
        let west_children = root_children["WestRedd"]["children"]
            .as_object()
            .expect("WestRedd should stay as a logical root");
        assert!(root_children.get("AVIAT_WestRedd").is_none());
        assert!(west_children.get("AVIAT_WestRedd").is_none());
        assert!(west_children.get("Tuscany Ridge").is_some());
    }


    #[test]
    fn hidden_native_root_remains_virtual_in_effective_tree() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;

        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-west".to_string(),
                    node_name: "WestRedd".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy:
                        TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "child-site".to_string(),
                    node_name: "Tuscany Ridge".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-west".to_string()),
                    current_parent_node_name: Some("WestRedd".to_string()),
                    current_attachment_id: Some("relay-b".to_string()),
                    current_attachment_name: Some("AVIAT_TuscanyRidge".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "site-west".to_string(),
                        parent_node_name: "WestRedd".to_string(),
                        attachment_options: vec![sample_attachment_option(
                            "relay-b",
                            "AVIAT_TuscanyRidge",
                        )],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let mut canonical = TopologyCanonicalStateFile::from_editor_and_network(
            &editor_state,
            &json!({
                "WestRedd": {
                    "children": {
                        "AVIAT_WestRedd": {
                            "children": {
                                "AVIAT_TuscanyRidge": {
                                    "children": {
                                        "Tuscany Ridge": {
                                            "children": {},
                                            "downloadBandwidthMbps": 900,
                                            "id": "child-site",
                                            "name": "Tuscany Ridge",
                                            "type": "Site",
                                            "uploadBandwidthMbps": 900
                                        }
                                    },
                                    "downloadBandwidthMbps": 900,
                                    "id": "relay-b",
                                    "name": "AVIAT_TuscanyRidge",
                                    "type": "AP",
                                    "uploadBandwidthMbps": 900
                                }
                            },
                            "downloadBandwidthMbps": 1000,
                            "id": "relay-a",
                            "name": "AVIAT_WestRedd",
                            "type": "AP",
                            "uploadBandwidthMbps": 1000
                        }
                    },
                    "downloadBandwidthMbps": 5000,
                    "id": "site-west",
                    "name": "WestRedd",
                    "type": "Site",
                    "uploadBandwidthMbps": 5000
                }
            }),
            TopologyCanonicalIngressKind::NativeIntegration,
        );
        canonical.nodes.push(TopologyCanonicalNode {
            node_id: "site-west".to_string(),
            node_name: "WestRedd".to_string(),
            latitude: None,
            longitude: None,
            node_kind: "Site".to_string(),
            is_virtual: false,
            current_parent_node_id: None,
            current_parent_node_name: None,
            current_attachment_id: None,
            current_attachment_name: None,
            can_move: false,
            allowed_parents: Vec::new(),
            queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren,
            rate_input: Default::default(),
        });

        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![
                TopologyEffectiveNodeState {
                    node_id: "site-west".to_string(),
                    logical_parent_node_id: String::new(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "child-site".to_string(),
                    logical_parent_node_id: "site-west".to_string(),
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
                },
            ],
        };

        let effective_network = apply_effective_topology_to_canonical_state(
            &config,
            &canonical,
            &editor_state,
            &effective,
            &QueueVirtualizationContext::default(),
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let west = root
            .get("WestRedd")
            .and_then(Value::as_object)
            .expect("WestRedd should remain visible as a logical virtual node");
        assert_eq!(west.get("virtual").and_then(Value::as_bool), Some(true));
        let west_children = west["children"]
            .as_object()
            .expect("WestRedd should retain its logical children");
        assert!(west_children.get("AVIAT_WestRedd").is_none());
        assert!(west_children.get("Tuscany Ridge").is_some());
    }


    #[test]
    fn large_visible_site_without_direct_circuits_stays_visible() {
        let (config, canonical, editor_state, effective) = site_with_ap_fixture();

        let effective_network = apply_effective_topology_to_canonical_state(
            &config,
            &canonical,
            &editor_state,
            &effective,
            &QueueVirtualizationContext::default(),
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let site = root["Aggregation"]
            .as_object()
            .expect("Aggregation should remain exported");

        assert_eq!(site.get("virtual").and_then(Value::as_bool), None);
    }


    #[test]
    fn large_site_with_direct_circuit_stays_visible() {
        let (config, canonical, editor_state, effective) = site_with_ap_fixture();
        let virtualization = QueueVirtualizationContext {
            direct_circuit_node_ids: HashSet::from(["site-agg".to_string()]),
            direct_circuit_node_names: HashSet::new(),
            forced_visible_node_names: HashSet::new(),
        };

        let canonical_network = canonical.insight_topology_network_json();
        let effective_network = apply_effective_topology_to_network_json_from_canonical(
            &config,
            &canonical_network,
            &canonical,
            &editor_state,
            &effective,
            &virtualization,
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let site = root["Aggregation"]
            .as_object()
            .expect("Aggregation should remain exported");

        assert_eq!(site.get("virtual").and_then(Value::as_bool), None);
    }


    #[test]
    fn large_site_with_name_only_direct_circuit_stays_visible() {
        let (config, canonical, editor_state, effective) = site_with_ap_fixture();
        let virtualization = QueueVirtualizationContext {
            direct_circuit_node_ids: HashSet::new(),
            direct_circuit_node_names: HashSet::from(["Aggregation".to_string()]),
            forced_visible_node_names: HashSet::new(),
        };

        let canonical_network = canonical.insight_topology_network_json();
        let effective_network = apply_effective_topology_to_network_json_from_canonical(
            &config,
            &canonical_network,
            &canonical,
            &editor_state,
            &effective,
            &virtualization,
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let site = root["Aggregation"]
            .as_object()
            .expect("Aggregation should remain exported");

        assert_eq!(site.get("virtual").and_then(Value::as_bool), None);
    }


    #[test]
    fn active_attachment_circuit_marks_owner_site_direct() {
        let shaped_devices = ConfigShapedDevices {
            devices: vec![ShapedDevice {
                circuit_id: "circuit-1".to_string(),
                circuit_name: "Circuit 1".to_string(),
                device_id: "device-1".to_string(),
                device_name: "Device 1".to_string(),
                parent_node: "Attachment Node".to_string(),
                parent_node_id: Some("attachment-node".to_string()),
                anchor_node_id: Some("attachment-node".to_string()),
                ..ShapedDevice::default()
            }],
            ..ConfigShapedDevices::default()
        };
        let attachment_owners = HashMap::from([(
            "attachment-node".to_string(),
            ("site-agg".to_string(), "Aggregation".to_string()),
        )]);

        let direct_node_ids =
            collect_direct_circuit_node_ids(&shaped_devices, &[], &attachment_owners);
        let direct_node_names = collect_direct_circuit_node_names(&shaped_devices, &[]);

        assert!(direct_node_ids.contains("attachment-node"));
        assert!(direct_node_ids.contains("site-agg"));
        assert!(direct_node_names.contains("Attachment Node"));
    }


    #[test]
    fn large_site_with_force_visible_override_stays_visible() {
        let (config, canonical, editor_state, effective) = site_with_ap_fixture();
        let virtualization = QueueVirtualizationContext {
            direct_circuit_node_ids: HashSet::new(),
            direct_circuit_node_names: HashSet::new(),
            forced_visible_node_names: HashSet::from(["Aggregation".to_string()]),
        };

        let canonical_network = canonical.insight_topology_network_json();
        let effective_network = apply_effective_topology_to_network_json_from_canonical(
            &config,
            &canonical_network,
            &canonical,
            &editor_state,
            &effective,
            &virtualization,
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let site = root["Aggregation"]
            .as_object()
            .expect("Aggregation should remain exported");

        assert_eq!(site.get("virtual").and_then(Value::as_bool), None);
    }


    #[test]
    fn queue_auto_marks_large_site_virtual_without_treeguard() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        config.topology.queue_auto_virtualize_threshold_mbps = 5_000;

        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-root".to_string(),
                    node_name: "Core".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "site-agg".to_string(),
                    node_name: "Aggregation".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Core".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueAuto,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "site-child".to_string(),
                    node_name: "Edge POP".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-agg".to_string()),
                    current_parent_node_name: Some("Aggregation".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let canonical = json!({
            "Core": {
                "children": {
                    "Aggregation": {
                        "children": {
                            "Edge POP": {
                                "children": {},
                                "downloadBandwidthMbps": 2000,
                                "id": "site-child",
                                "name": "Edge POP",
                                "type": "Site",
                                "uploadBandwidthMbps": 2000
                            }
                        },
                        "downloadBandwidthMbps": 7000,
                        "id": "site-agg",
                        "name": "Aggregation",
                        "type": "Site",
                        "uploadBandwidthMbps": 7000
                    }
                },
                "downloadBandwidthMbps": 20000,
                "id": "site-root",
                "name": "Core",
                "type": "Site",
                "uploadBandwidthMbps": 20000
            }
        });

        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![
                TopologyEffectiveNodeState {
                    node_id: "site-root".to_string(),
                    logical_parent_node_id: String::new(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "site-agg".to_string(),
                    logical_parent_node_id: "site-root".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "site-child".to_string(),
                    logical_parent_node_id: "site-agg".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
            ],
        };

        let effective_network = apply_effective_topology_to_network_json(
            &config,
            &canonical,
            &editor_state,
            &effective,
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let core = root["Core"]
            .as_object()
            .expect("Core should remain exported");
        let core_children = core["children"]
            .as_object()
            .expect("Core should remain exported");
        let aggregation = core_children
            .get("Aggregation")
            .and_then(Value::as_object)
            .expect("Aggregation should remain visible as a virtual node");
        assert_eq!(
            aggregation.get("virtual").and_then(Value::as_bool),
            Some(true)
        );
        let aggregation_children = aggregation["children"]
            .as_object()
            .expect("Aggregation should retain its logical children");
        assert!(aggregation_children.get("Edge POP").is_some());
    }

    fn ap_branch_fixture() -> (
        Config,
        Value,
        TopologyEditorStateFile,
        TopologyEffectiveStateFile,
    ) {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        config.topology.queue_auto_virtualize_threshold_mbps = 5_000;

        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-root".to_string(),
                    node_name: "Core".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "ap-agg".to_string(),
                    node_name: "Aggregation Switch".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Core".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueAuto,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "ap-child".to_string(),
                    node_name: "Access AP".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("ap-agg".to_string()),
                    current_parent_node_name: Some("Aggregation Switch".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let canonical = json!({
            "Core": {
                "children": {
                    "Aggregation Switch": {
                        "children": {
                            "Access AP": {
                                "children": {},
                                "downloadBandwidthMbps": 1000,
                                "id": "ap-child",
                                "name": "Access AP",
                                "type": "AP",
                                "uploadBandwidthMbps": 1000
                            }
                        },
                        "downloadBandwidthMbps": 10000,
                        "id": "ap-agg",
                        "name": "Aggregation Switch",
                        "type": "AP",
                        "uploadBandwidthMbps": 10000
                    }
                },
                "downloadBandwidthMbps": 20000,
                "id": "site-root",
                "name": "Core",
                "type": "Site",
                "uploadBandwidthMbps": 20000
            }
        });

        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![
                TopologyEffectiveNodeState {
                    node_id: "site-root".to_string(),
                    logical_parent_node_id: String::new(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "ap-agg".to_string(),
                    logical_parent_node_id: "site-root".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "ap-child".to_string(),
                    logical_parent_node_id: "ap-agg".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
            ],
        };

        (config, canonical, editor_state, effective)
    }


    #[test]
    fn queue_auto_marks_large_ap_branch_virtual_without_treeguard() {
        let (config, canonical, editor_state, effective) = ap_branch_fixture();

        let effective_network = apply_effective_topology_to_network_json(
            &config,
            &canonical,
            &editor_state,
            &effective,
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let aggregation = root["Core"]["children"]["Aggregation Switch"]
            .as_object()
            .expect("Aggregation Switch should remain visible as a virtual node");
        assert_eq!(
            aggregation.get("virtual").and_then(Value::as_bool),
            Some(true)
        );
        let aggregation_children = aggregation["children"]
            .as_object()
            .expect("Aggregation Switch should retain its logical children");
        assert!(aggregation_children.get("Access AP").is_some());
    }


    #[test]
    fn queue_auto_marks_large_ap_branch_virtual_with_effective_tree_children() {
        let (config, canonical, mut editor_state, mut effective) = ap_branch_fixture();
        let access_ap = editor_state
            .nodes
            .iter_mut()
            .find(|node| node.node_id == "ap-child")
            .expect("fixture should include Access AP");
        access_ap.current_parent_node_id = Some("site-root".to_string());
        access_ap.current_parent_node_name = Some("Core".to_string());
        let access_ap_effective = effective
            .nodes
            .iter_mut()
            .find(|node| node.node_id == "ap-child")
            .expect("fixture should include Access AP effective state");
        access_ap_effective.logical_parent_node_id = "site-root".to_string();
        access_ap_effective.effective_attachment_id = Some("ap-agg".to_string());
        access_ap_effective.attachments = vec![TopologyEffectiveAttachmentState {
            attachment_id: "ap-agg".to_string(),
            effective_selected: true,
            health_reason: None,
            health_status: TopologyAttachmentHealthStatus::Healthy,
            probe_enabled: false,
            probeable: false,
            suppressed_until_unix: None,
        }];

        let effective_network = apply_effective_topology_to_network_json(
            &config,
            &canonical,
            &editor_state,
            &effective,
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let aggregation = root["Core"]["children"]["Aggregation Switch"]
            .as_object()
            .expect("Aggregation Switch should remain visible as a virtual node");

        assert_eq!(
            aggregation.get("virtual").and_then(Value::as_bool),
            Some(true)
        );
    }


    #[test]
    fn queue_auto_keeps_large_ap_branch_visible_with_direct_circuit() {
        let (config, canonical, editor_state, effective) = ap_branch_fixture();
        let virtualization = QueueVirtualizationContext {
            direct_circuit_node_ids: HashSet::from(["ap-agg".to_string()]),
            direct_circuit_node_names: HashSet::new(),
            forced_visible_node_names: HashSet::new(),
        };
        let canonical_state = TopologyCanonicalStateFile::from_editor_and_network(
            &editor_state,
            &canonical,
            TopologyCanonicalIngressKind::NativeIntegration,
        );

        let effective_network = apply_effective_topology_to_network_json_from_canonical(
            &config,
            &canonical,
            &canonical_state,
            &editor_state,
            &effective,
            &virtualization,
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let aggregation = root["Core"]["children"]["Aggregation Switch"]
            .as_object()
            .expect("Aggregation Switch should remain exported");

        assert_eq!(aggregation.get("virtual").and_then(Value::as_bool), None);
    }


    #[test]
    fn queue_auto_uses_recompiled_effective_rate_before_virtualizing() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        config.topology.queue_auto_virtualize_threshold_mbps = 5_000;

        let editor_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-root".to_string(),
                    node_name: "Root".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "site-agg".to_string(),
                    node_name: "Aggregation".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Root".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueAuto,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "site-child".to_string(),
                    node_name: "Edge".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-agg".to_string()),
                    current_parent_node_name: Some("Aggregation".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: Vec::new(),
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let canonical = json!({
            "Root": {
                "children": {
                    "Aggregation": {
                        "children": {
                            "Edge": {
                                "children": {},
                                "downloadBandwidthMbps": 1000,
                                "id": "site-child",
                                "name": "Edge",
                                "type": "Site",
                                "uploadBandwidthMbps": 1000
                            }
                        },
                        "downloadBandwidthMbps": 100000,
                        "id": "site-agg",
                        "name": "Aggregation",
                        "type": "Site",
                        "uploadBandwidthMbps": 100000
                    }
                },
                "downloadBandwidthMbps": 2350,
                "id": "site-root",
                "name": "Root",
                "type": "Site",
                "uploadBandwidthMbps": 2350
            }
        });

        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![
                TopologyEffectiveNodeState {
                    node_id: "site-root".to_string(),
                    logical_parent_node_id: String::new(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "site-agg".to_string(),
                    logical_parent_node_id: "site-root".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "site-child".to_string(),
                    logical_parent_node_id: "site-agg".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
            ],
        };

        let effective_network = apply_effective_topology_to_network_json(
            &config,
            &canonical,
            &editor_state,
            &effective,
        );
        let root = effective_network
            .as_object()
            .expect("effective export should remain an object tree");
        let aggregation = root["Root"]["children"]["Aggregation"]
            .as_object()
            .expect("Aggregation should remain exported");
        assert_eq!(
            aggregation
                .get("downloadBandwidthMbps")
                .and_then(Value::as_u64),
            Some(2350)
        );
        assert_eq!(
            aggregation
                .get("uploadBandwidthMbps")
                .and_then(Value::as_u64),
            Some(2350)
        );
        assert_eq!(aggregation.get("virtual").and_then(Value::as_bool), None);
    }
