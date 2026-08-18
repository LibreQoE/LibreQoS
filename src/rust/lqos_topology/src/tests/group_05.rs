    #[test]
    fn duplicate_device_candidates_do_not_block_valid_site_override_publish() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;

        let canonical_network = json!({
            "Site Alpha": {
                "children": {
                    "Alpha-Beta-60": {
                        "children": {},
                        "downloadBandwidthMbps": 940,
                        "id": "alpha-beta-60",
                        "name": "Alpha-Beta-60",
                        "parent_site": "Site Alpha",
                        "type": "AP",
                        "uploadBandwidthMbps": 940
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "site-alpha",
                "name": "Site Alpha",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            },
            "Site Beta": {
                "children": {
                    "Beta - Alpha 60": {
                        "children": {},
                        "downloadBandwidthMbps": 940,
                        "id": "beta-alpha-60",
                        "name": "Beta - Alpha 60",
                        "parent_site": "Site Beta",
                        "type": "AP",
                        "uploadBandwidthMbps": 940
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "site-beta",
                "name": "Site Beta",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });

        let mut beta_alpha_option = sample_attachment_option("beta-alpha-60", "Beta - Alpha 60");
        beta_alpha_option.peer_attachment_id = Some("alpha-beta-60".to_string());
        beta_alpha_option.peer_attachment_name = Some("Alpha-Beta-60".to_string());
        beta_alpha_option.download_bandwidth_mbps = Some(940);
        beta_alpha_option.upload_bandwidth_mbps = Some(940);
        beta_alpha_option.capacity_mbps = Some(940);

        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-alpha".to_string(),
                    node_name: "Site Alpha".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "site-beta".to_string(),
                    node_name: "Site Beta".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-gamma".to_string()),
                    current_parent_node_name: Some("Site Gamma".to_string()),
                    current_attachment_id: Some("beta-gamma-60".to_string()),
                    current_attachment_name: Some("Beta - Gamma 60".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "site-alpha".to_string(),
                        parent_node_name: "Site Alpha".to_string(),
                        attachment_options: vec![beta_alpha_option.clone()],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "beta-alpha-60".to_string(),
                    node_name: "Beta - Alpha 60".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-beta".to_string()),
                    current_parent_node_name: Some("Site Beta".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "site-beta".to_string(),
                        parent_node_name: "Site Beta".to_string(),
                        attachment_options: vec![],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    }],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "beta-alpha-60".to_string(),
                    node_name: "Beta - Alpha 60".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-beta".to_string()),
                    current_parent_node_name: Some("Site Beta".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![
                        TopologyAllowedParent {
                            parent_node_id: "site-alpha".to_string(),
                            parent_node_name: "Site Alpha".to_string(),
                            attachment_options: vec![beta_alpha_option.clone()],
                            all_attachments_suppressed: false,
                            has_probe_unavailable_attachments: false,
                        },
                        TopologyAllowedParent {
                            parent_node_id: "site-beta".to_string(),
                            parent_node_name: "Site Beta".to_string(),
                            attachment_options: vec![],
                            all_attachments_suppressed: false,
                            has_probe_unavailable_attachments: false,
                        },
                    ],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let mut overrides = TopologyOverridesFile::default();
        overrides.set_override_return_changed(
            "site-beta".to_string(),
            "Site Beta".to_string(),
            "site-alpha".to_string(),
            "Site Alpha".to_string(),
            TopologyAttachmentMode::Auto,
            Vec::new(),
        );

        let artifacts = build_effective_topology_artifacts(
            &config,
            &canonical,
            &overrides,
            &TopologyAttachmentHealthStateFile::default(),
            Some(&canonical_network),
        )
        .expect("duplicate device candidates should normalize before validation");

        assert_eq!(
            artifacts
                .effective
                .nodes
                .iter()
                .filter(|node| node.node_id == "beta-alpha-60")
                .count(),
            1
        );
        let moved = artifacts
            .effective_network
            .expect("effective network should be published");
        assert!(moved.is_object());
    }


    #[test]
    fn effective_topology_validation_rejects_missing_site() {
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "plain-beta".to_string(),
                node_name: "Site Beta".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("plain-alpha".to_string()),
                current_parent_node_name: Some("Site Alpha".to_string()),
                current_attachment_id: Some("beta-alpha-60".to_string()),
                current_attachment_name: Some("Beta - Alpha 60".to_string()),
                can_move: true,
                allowed_parents: vec![],
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
                node_id: "plain-beta".to_string(),
                logical_parent_node_id: "plain-alpha".to_string(),
                preferred_attachment_id: Some("beta-alpha-60".to_string()),
                effective_attachment_id: Some("beta-alpha-60".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![],
            }],
        };
        let exported = json!({
            "Site Alpha": {
                "children": {},
                "id": "plain-alpha",
                "name": "Site Alpha",
                "type": "Site"
            }
        });

        let config = Config::default();
        let canonical_network = json!({
            "Site Alpha": {
                "children": {
                    "Site Beta": {
                        "children": {},
                        "id": "plain-beta",
                        "name": "Site Beta",
                        "type": "Site"
                    }
                },
                "id": "plain-alpha",
                "name": "Site Alpha",
                "type": "Site"
            }
        });

        let errors = validate_effective_topology_network(
            &config,
            &canonical_network,
            &ui_state,
            &effective,
            &exported,
        )
        .expect_err("missing site should fail validation");
        assert!(errors.iter().any(|error| error.contains("Site Beta")));
    }


    #[test]
    fn effective_topology_validation_rejects_site_parent_cycles() {
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "plain-a".to_string(),
                    node_name: "Site A".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("plain-b".to_string()),
                    current_parent_node_name: Some("Site B".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "plain-b".to_string(),
                    node_name: "Site B".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("plain-a".to_string()),
                    current_parent_node_name: Some("Site A".to_string()),
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: true,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };
        let effective = TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![
                TopologyEffectiveNodeState {
                    node_id: "plain-a".to_string(),
                    logical_parent_node_id: "plain-b".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
                TopologyEffectiveNodeState {
                    node_id: "plain-b".to_string(),
                    logical_parent_node_id: "plain-a".to_string(),
                    preferred_attachment_id: None,
                    effective_attachment_id: None,
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
            ],
        };
        let exported = json!({
            "Site A": {
                "children": {},
                "id": "plain-a",
                "name": "Site A",
                "type": "Site"
            },
            "Site B": {
                "children": {},
                "id": "plain-b",
                "name": "Site B",
                "type": "Site"
            }
        });

        let config = Config::default();
        let errors = validate_effective_topology_network(
            &config, &exported, &ui_state, &effective, &exported,
        )
        .expect_err("site-parent cycle should fail validation");
        assert!(errors.iter().any(|error| error.contains("parent cycle")));
    }


    #[test]
    fn effective_topology_validation_rejects_invalid_attachment_for_selected_parent() {
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "uisp:site:site-beta".to_string(),
                node_name: "Site Beta".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("uisp:site:site-alpha".to_string()),
                current_parent_node_name: Some("Site Alpha".to_string()),
                current_attachment_id: Some("alpha-beta-60".to_string()),
                current_attachment_name: Some("Alpha-Beta-60".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "uisp:site:site-alpha".to_string(),
                    parent_node_name: "Site Alpha".to_string(),
                    attachment_options: vec![sample_attachment_option(
                        "alpha-beta-60",
                        "Alpha-Beta-60",
                    )],
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
                node_id: "uisp:site:site-beta".to_string(),
                logical_parent_node_id: "uisp:site:site-alpha".to_string(),
                preferred_attachment_id: Some("alpha-beta-60".to_string()),
                effective_attachment_id: Some("beta-alpha-60".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![],
            }],
        };
        let exported = json!({
            "Site Alpha": {
                "children": {
                    "Site Beta": {
                        "children": {},
                        "id": "uisp:site:site-beta",
                        "name": "Site Beta",
                        "type": "Site"
                    }
                },
                "id": "uisp:site:site-alpha",
                "name": "Site Alpha",
                "type": "Site"
            }
        });

        let config = Config::default();
        let errors = validate_effective_topology_network(
            &config, &exported, &ui_state, &effective, &exported,
        )
        .expect_err("invalid attachment should fail validation");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("invalid attachment"))
        );
    }


    #[test]
    fn effective_topology_validation_accepts_fixed_parent_nodes_without_allowed_parents() {
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/ap_site".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-root".to_string(),
                    node_name: "Site Root".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "ap-child".to_string(),
                    node_name: "AP Child".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Site Root".to_string()),
                    current_attachment_id: Some("legacy-attachment".to_string()),
                    current_attachment_name: Some("Legacy Attachment".to_string()),
                    can_move: false,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };
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
                    node_id: "ap-child".to_string(),
                    logical_parent_node_id: "site-root".to_string(),
                    preferred_attachment_id: Some("legacy-attachment".to_string()),
                    effective_attachment_id: Some("legacy-attachment".to_string()),
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![],
                },
            ],
        };
        let exported = json!({
            "Site Root": {
                "children": {
                    "AP Child": {
                        "children": {},
                        "id": "ap-child",
                        "name": "AP Child",
                        "parent_site": "Site Root",
                        "type": "AP"
                    }
                },
                "id": "site-root",
                "name": "Site Root",
                "type": "Site"
            }
        });

        let config = Config::default();
        validate_effective_topology_network(&config, &exported, &ui_state, &effective, &exported)
            .expect("fixed-parent legacy nodes should validate");
    }


    #[test]
    fn compute_effective_state_keeps_fixed_parent_nodes_without_allowed_parents() {
        let config = Config::default();
        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/ap_site".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![
                TopologyEditorNode {
                    node_id: "site-root".to_string(),
                    node_name: "Site Root".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
                    current_attachment_id: None,
                    current_attachment_name: None,
                    can_move: false,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
                TopologyEditorNode {
                    node_id: "ap-child".to_string(),
                    node_name: "AP Child".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Site Root".to_string()),
                    current_attachment_id: Some("legacy-attachment".to_string()),
                    current_attachment_name: Some("Legacy Attachment".to_string()),
                    can_move: false,
                    allowed_parents: vec![],
                    queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                    preferred_attachment_id: None,
                    preferred_attachment_name: None,
                    effective_attachment_id: None,
                    effective_attachment_name: None,
                },
            ],
        };

        let effective = compute_effective_state(
            &config,
            &canonical,
            &TopologyOverridesFile::default(),
            &TopologyAttachmentHealthStateFile::default(),
        );

        assert_eq!(effective.nodes.len(), 2);
        let child = effective
            .nodes
            .iter()
            .find(|node| node.node_id == "ap-child")
            .expect("child node should remain in effective state");
        assert_eq!(child.logical_parent_node_id, "site-root");
        assert_eq!(
            child.preferred_attachment_id.as_deref(),
            Some("legacy-attachment")
        );
        assert_eq!(
            child.effective_attachment_id.as_deref(),
            Some("legacy-attachment")
        );
        assert!(child.attachments.is_empty());
        assert!(!child.all_attachments_suppressed);
    }


    #[test]
    fn compute_effective_state_does_not_infer_parent_for_native_integration_nodes() {
        let config = Config::default();
        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "python/full".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "splynx:site:child".to_string(),
                node_name: "Child Site".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: None,
                current_parent_node_name: None,
                current_attachment_id: None,
                current_attachment_name: None,
                can_move: false,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "splynx:site:parent".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![],
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

        let effective = compute_effective_state(
            &config,
            &canonical,
            &TopologyOverridesFile::default(),
            &TopologyAttachmentHealthStateFile::default(),
        );

        assert_eq!(effective.nodes.len(), 1);
        let child = &effective.nodes[0];
        assert_eq!(child.node_id, "splynx:site:child");
        assert!(child.logical_parent_node_id.is_empty());
        assert!(child.preferred_attachment_id.is_none());
        assert!(child.effective_attachment_id.is_none());
        assert!(child.attachments.is_empty());
    }


    #[test]
    fn compute_effective_state_auto_prefers_dynamic_attachment_when_probes_disabled() {
        let config = Config::default();
        let mut dynamic_attachment =
            sample_attachment_option("dynamic-link", "WavePro-MREToRochester");
        dynamic_attachment.rate_source = TopologyAttachmentRateSource::DynamicIntegration;
        dynamic_attachment.capacity_mbps = Some(2700);
        dynamic_attachment.download_bandwidth_mbps = Some(2700);
        dynamic_attachment.upload_bandwidth_mbps = Some(2700);
        dynamic_attachment.local_probe_ip = Some("100.126.0.226".to_string());
        dynamic_attachment.probe_enabled = false;

        let mut static_attachment = sample_attachment_option("static-link", "4600C_MRE_To_ROCH");
        static_attachment.rate_source = TopologyAttachmentRateSource::Static;
        static_attachment.capacity_mbps = Some(8000);
        static_attachment.download_bandwidth_mbps = Some(8000);
        static_attachment.upload_bandwidth_mbps = Some(8000);
        static_attachment.local_probe_ip = Some("100.126.0.235".to_string());
        static_attachment.remote_probe_ip = Some("100.126.0.234".to_string());
        static_attachment.probe_enabled = false;

        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "site-mre".to_string(),
                node_name: "MRE".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("site-rochester".to_string()),
                current_parent_node_name: Some("7232 Rochester".to_string()),
                current_attachment_id: Some("static-link".to_string()),
                current_attachment_name: Some("4600C_MRE_To_ROCH".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "site-rochester".to_string(),
                    parent_node_name: "7232 Rochester".to_string(),
                    attachment_options: vec![
                        auto_attachment_option(),
                        dynamic_attachment,
                        static_attachment,
                    ],
                    all_attachments_suppressed: false,
                    has_probe_unavailable_attachments: false,
                }],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueAuto,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };

        let effective = compute_effective_state(
            &config,
            &canonical,
            &TopologyOverridesFile::default(),
            &TopologyAttachmentHealthStateFile::default(),
        );

        let node = effective
            .nodes
            .iter()
            .find(|node| node.node_id == "site-mre")
            .expect("MRE node should remain in effective state");
        assert_eq!(
            node.preferred_attachment_id.as_deref(),
            Some("dynamic-link")
        );
        assert_eq!(
            node.effective_attachment_id.as_deref(),
            Some("dynamic-link")
        );
        assert!(node.fallback_reason.is_none());
    }


    #[test]
    fn effective_state_fallback_does_not_keep_old_parent_attachment_after_reparent() {
        use lqos_overrides::TopologyOverridesFile;

        let config = Config::default();
        let canonical = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "uisp:site:site-beta".to_string(),
                node_name: "Site Beta".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("uisp:site:site-gamma".to_string()),
                current_parent_node_name: Some("Site Gamma".to_string()),
                current_attachment_id: Some("uisp:device:device-beta-gamma".to_string()),
                current_attachment_name: Some("Beta - Gamma MLO6".to_string()),
                can_move: true,
                allowed_parents: vec![
                    TopologyAllowedParent {
                        parent_node_id: "uisp:site:site-alpha".to_string(),
                        parent_node_name: "Site Alpha".to_string(),
                        attachment_options: vec![
                            TopologyAttachmentOption {
                                attachment_id: "auto".to_string(),
                                attachment_name: "Auto".to_string(),
                                attachment_kind: "auto".to_string(),
                                attachment_role: TopologyAttachmentRole::Unknown,
                                pair_id: None,
                                peer_attachment_id: None,
                                peer_attachment_name: None,
                                capacity_mbps: None,
                                download_bandwidth_mbps: None,
                                upload_bandwidth_mbps: None,
                                transport_cap_mbps: None,
                                transport_cap_reason: None,
                                rate_source: TopologyAttachmentRateSource::Unknown,
                                can_override_rate: false,
                                rate_override_disabled_reason: None,
                                has_rate_override: false,
                                local_probe_ip: None,
                                remote_probe_ip: None,
                                probe_enabled: false,
                                probeable: false,
                                health_status: TopologyAttachmentHealthStatus::Disabled,
                                health_reason: None,
                                suppressed_until_unix: None,
                                effective_selected: false,
                            },
                            TopologyAttachmentOption {
                                attachment_id: "uisp:device:device-beta-alpha".to_string(),
                                attachment_name: "Beta - Alpha 60".to_string(),
                                attachment_kind: "device".to_string(),
                                attachment_role: TopologyAttachmentRole::PtpBackhaul,
                                pair_id: None,
                                peer_attachment_id: Some(
                                    "uisp:device:device-alpha-beta".to_string(),
                                ),
                                peer_attachment_name: Some("Alpha-Beta-60".to_string()),
                                capacity_mbps: Some(940),
                                download_bandwidth_mbps: Some(940),
                                upload_bandwidth_mbps: Some(940),
                                transport_cap_mbps: None,
                                transport_cap_reason: None,
                                rate_source: TopologyAttachmentRateSource::DynamicIntegration,
                                can_override_rate: false,
                                rate_override_disabled_reason: None,
                                has_rate_override: false,
                                local_probe_ip: Some("10.1.11.126".to_string()),
                                remote_probe_ip: Some("10.1.11.125".to_string()),
                                probe_enabled: false,
                                probeable: false,
                                health_status: TopologyAttachmentHealthStatus::Disabled,
                                health_reason: None,
                                suppressed_until_unix: None,
                                effective_selected: false,
                            },
                        ],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    },
                    TopologyAllowedParent {
                        parent_node_id: "uisp:site:site-gamma".to_string(),
                        parent_node_name: "Site Gamma".to_string(),
                        attachment_options: vec![
                            TopologyAttachmentOption {
                                attachment_id: "auto".to_string(),
                                attachment_name: "Auto".to_string(),
                                attachment_kind: "auto".to_string(),
                                attachment_role: TopologyAttachmentRole::Unknown,
                                pair_id: None,
                                peer_attachment_id: None,
                                peer_attachment_name: None,
                                capacity_mbps: None,
                                download_bandwidth_mbps: None,
                                upload_bandwidth_mbps: None,
                                transport_cap_mbps: None,
                                transport_cap_reason: None,
                                rate_source: TopologyAttachmentRateSource::Unknown,
                                can_override_rate: false,
                                rate_override_disabled_reason: None,
                                has_rate_override: false,
                                local_probe_ip: None,
                                remote_probe_ip: None,
                                probe_enabled: false,
                                probeable: false,
                                health_status: TopologyAttachmentHealthStatus::Disabled,
                                health_reason: None,
                                suppressed_until_unix: None,
                                effective_selected: false,
                            },
                            TopologyAttachmentOption {
                                attachment_id: "uisp:device:device-beta-gamma".to_string(),
                                attachment_name: "Beta - Gamma MLO6".to_string(),
                                attachment_kind: "device".to_string(),
                                attachment_role: TopologyAttachmentRole::PtpBackhaul,
                                pair_id: None,
                                peer_attachment_id: Some(
                                    "uisp:device:device-gamma-beta".to_string(),
                                ),
                                peer_attachment_name: Some("Gamma - Beta MLO6".to_string()),
                                capacity_mbps: Some(230),
                                download_bandwidth_mbps: Some(230),
                                upload_bandwidth_mbps: Some(230),
                                transport_cap_mbps: None,
                                transport_cap_reason: None,
                                rate_source: TopologyAttachmentRateSource::DynamicIntegration,
                                can_override_rate: false,
                                rate_override_disabled_reason: None,
                                has_rate_override: false,
                                local_probe_ip: Some("10.1.33.23".to_string()),
                                remote_probe_ip: Some("10.1.33.21".to_string()),
                                probe_enabled: false,
                                probeable: false,
                                health_status: TopologyAttachmentHealthStatus::Disabled,
                                health_reason: None,
                                suppressed_until_unix: None,
                                effective_selected: false,
                            },
                        ],
                        all_attachments_suppressed: false,
                        has_probe_unavailable_attachments: false,
                    },
                ],
                queue_visibility_policy: TopologyQueueVisibilityPolicy::QueueVisible,
                preferred_attachment_id: None,
                preferred_attachment_name: None,
                effective_attachment_id: None,
                effective_attachment_name: None,
            }],
        };
        let mut overrides = TopologyOverridesFile::default();
        overrides.set_override_return_changed(
            "uisp:site:site-beta".to_string(),
            "Site Beta".to_string(),
            "uisp:site:site-alpha".to_string(),
            "Site Alpha".to_string(),
            TopologyAttachmentMode::Auto,
            Vec::new(),
        );

        let effective = compute_effective_state(
            &config,
            &canonical,
            &overrides,
            &TopologyAttachmentHealthStateFile::default(),
        );
        let node = effective
            .nodes
            .iter()
            .find(|node| node.node_id == "uisp:site:site-beta")
            .expect("expected Site Beta effective state");

        assert_eq!(node.logical_parent_node_id, "uisp:site:site-alpha");
        assert_eq!(
            node.effective_attachment_id.as_deref(),
            Some("uisp:device:device-beta-alpha")
        );
        assert_ne!(
            node.effective_attachment_id.as_deref(),
            Some("uisp:device:device-beta-gamma")
        );
    }
