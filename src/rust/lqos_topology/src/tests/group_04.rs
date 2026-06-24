    #[test]
    fn queue_auto_top_level_site_below_threshold_stays_visible() {
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
                    node_name: "Root Aggregation".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
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
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Root Aggregation".to_string()),
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
            "Root Aggregation": {
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
                "downloadBandwidthMbps": 2350,
                "id": "site-root",
                "name": "Root Aggregation",
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
                    node_id: "site-child".to_string(),
                    logical_parent_node_id: "site-root".to_string(),
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
        let root_node = root
            .get("Root Aggregation")
            .and_then(Value::as_object)
            .expect("root node should remain exported");
        assert_eq!(root_node.get("virtual").and_then(Value::as_bool), None);
        let children = root_node["children"]
            .as_object()
            .expect("root node should retain its logical children");
        assert!(children.get("Edge").is_some());
    }


    #[test]
    fn queue_auto_top_level_site_above_threshold_becomes_virtual() {
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
                    node_name: "Root Aggregation".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: None,
                    current_parent_node_name: None,
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
                    current_parent_node_id: Some("site-root".to_string()),
                    current_parent_node_name: Some("Root Aggregation".to_string()),
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
            "Root Aggregation": {
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
                "downloadBandwidthMbps": 7000,
                "id": "site-root",
                "name": "Root Aggregation",
                "type": "Site",
                "uploadBandwidthMbps": 7000
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
                    node_id: "site-child".to_string(),
                    logical_parent_node_id: "site-root".to_string(),
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
        let root_node = root
            .get("Root Aggregation")
            .and_then(Value::as_object)
            .expect("root node should remain exported");
        assert_eq!(
            root_node.get("virtual").and_then(Value::as_bool),
            Some(true)
        );
        let children = root_node["children"]
            .as_object()
            .expect("root node should retain its logical children");
        assert!(children.get("Edge").is_some());
    }


    #[test]
    fn runtime_squashing_respects_do_not_squash_sites() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        config.uisp_integration.do_not_squash_sites = Some(vec!["Child Site".to_string()]);
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
            nodes: Vec::new(),
        };
        let effective = TopologyEffectiveStateFile::default();

        let squashed =
            apply_effective_topology_to_network_json(&config, &canonical, &ui_state, &effective);
        assert!(squashed["Parent Site"]["children"]["Relay A"].is_object());
        assert!(squashed["Parent Site"]["children"]["Child Site"].is_null());
    }


    #[test]
    fn runtime_squashing_keeps_ptmp_uplink_aps_visible() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Access AP": {
                        "children": {
                            "Child CPE": {
                                "children": {
                                    "Child Site": {
                                        "children": {},
                                        "downloadBandwidthMbps": 110,
                                        "id": "child-site",
                                        "name": "Child Site",
                                        "parent_site": "Child CPE",
                                        "type": "Site",
                                        "uploadBandwidthMbps": 30
                                    }
                                },
                                "downloadBandwidthMbps": 209,
                                "id": "child-cpe",
                                "name": "Child CPE",
                                "parent_site": "Access AP",
                                "type": "AP",
                                "uploadBandwidthMbps": 40
                            }
                        },
                        "downloadBandwidthMbps": 313,
                        "id": "parent-ap",
                        "name": "Access AP",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 64
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });
        let mut ptmp_attachment = sample_attachment_option("child-cpe", "Child CPE");
        ptmp_attachment.attachment_role = TopologyAttachmentRole::PtmpUplink;
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
                current_attachment_id: Some("child-cpe".to_string()),
                current_attachment_name: Some("Child CPE".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "parent-site".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![ptmp_attachment],
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
                preferred_attachment_id: Some("child-cpe".to_string()),
                effective_attachment_id: Some("child-cpe".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![TopologyEffectiveAttachmentState {
                    attachment_id: "child-cpe".to_string(),
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
        assert!(
            parent_children
                .get("Access AP")
                .and_then(|value| value.as_object())
                .is_some()
        );
        assert!(parent_children.get("Child Site").is_none());
    }


    #[test]
    fn effective_export_keeps_logical_children_without_explicit_attachment() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Leaf AP": {
                        "children": {},
                        "downloadBandwidthMbps": 150,
                        "id": "leaf-ap",
                        "name": "Leaf AP",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 75
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
            source: "uisp/full".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "leaf-ap".to_string(),
                node_name: "Leaf AP".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("parent-site".to_string()),
                current_parent_node_name: Some("Parent Site".to_string()),
                current_attachment_id: Some("parent-site".to_string()),
                current_attachment_name: Some("Parent Site".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "parent-site".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![auto_attachment_option()],
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
                node_id: "leaf-ap".to_string(),
                logical_parent_node_id: "parent-site".to_string(),
                preferred_attachment_id: None,
                effective_attachment_id: None,
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![],
            }],
        };

        let exported =
            apply_effective_topology_to_network_json(&config, &canonical, &ui_state, &effective);
        let parent_children = exported["Parent Site"]["children"]
            .as_object()
            .expect("parent should keep children");
        assert!(
            parent_children
                .get("Leaf AP")
                .and_then(Value::as_object)
                .is_some()
        );
    }


    #[test]
    fn runtime_prunes_inactive_backhaul_attachment_stubs() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
            "Parent Site": {
                "children": {
                    "Active Parent Attachment": {
                        "children": {
                            "Active Child Attachment": {
                                "children": {
                                    "Child Site": {
                                        "children": {},
                                        "downloadBandwidthMbps": 900,
                                        "id": "child-site",
                                        "name": "Child Site",
                                        "parent_site": "Active Child Attachment",
                                        "type": "Site",
                                        "uploadBandwidthMbps": 900
                                    }
                                },
                                "downloadBandwidthMbps": 400,
                                "id": "active-child-attachment",
                                "name": "Active Child Attachment",
                                "parent_site": "Active Parent Attachment",
                                "type": "AP",
                                "uploadBandwidthMbps": 400
                            }
                        },
                        "downloadBandwidthMbps": 400,
                        "id": "active-parent-attachment",
                        "name": "Active Parent Attachment",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 400
                    },
                    "Inactive Parent Attachment": {
                        "children": {
                            "Inactive Child Attachment": {
                                "children": {},
                                "downloadBandwidthMbps": 2350,
                                "id": "inactive-child-attachment",
                                "name": "Inactive Child Attachment",
                                "parent_site": "Inactive Parent Attachment",
                                "type": "AP",
                                "uploadBandwidthMbps": 2350
                            }
                        },
                        "downloadBandwidthMbps": 2350,
                        "id": "inactive-parent-attachment",
                        "name": "Inactive Parent Attachment",
                        "parent_site": "Parent Site",
                        "type": "AP",
                        "uploadBandwidthMbps": 2350
                    }
                },
                "downloadBandwidthMbps": 1000,
                "id": "parent-site",
                "name": "Parent Site",
                "type": "Site",
                "uploadBandwidthMbps": 1000
            }
        });

        let mut active_attachment =
            sample_attachment_option("active-child-attachment", "Active Child Attachment");
        active_attachment.pair_id =
            Some("active-child-attachment|active-parent-attachment".to_string());
        active_attachment.peer_attachment_id = Some("active-parent-attachment".to_string());
        active_attachment.peer_attachment_name = Some("Active Parent Attachment".to_string());
        active_attachment.capacity_mbps = Some(400);
        active_attachment.download_bandwidth_mbps = Some(400);
        active_attachment.upload_bandwidth_mbps = Some(400);

        let mut inactive_attachment =
            sample_attachment_option("inactive-child-attachment", "Inactive Child Attachment");
        inactive_attachment.pair_id =
            Some("inactive-child-attachment|inactive-parent-attachment".to_string());
        inactive_attachment.peer_attachment_id = Some("inactive-parent-attachment".to_string());
        inactive_attachment.peer_attachment_name = Some("Inactive Parent Attachment".to_string());
        inactive_attachment.capacity_mbps = Some(2350);
        inactive_attachment.download_bandwidth_mbps = Some(2350);
        inactive_attachment.upload_bandwidth_mbps = Some(2350);

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
                current_attachment_id: Some("active-child-attachment".to_string()),
                current_attachment_name: Some("Active Child Attachment".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "parent-site".to_string(),
                    parent_node_name: "Parent Site".to_string(),
                    attachment_options: vec![active_attachment, inactive_attachment],
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
                preferred_attachment_id: Some("active-child-attachment".to_string()),
                effective_attachment_id: Some("active-child-attachment".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![TopologyEffectiveAttachmentState {
                    attachment_id: "active-child-attachment".to_string(),
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
        assert!(parent_children.get("Inactive Parent Attachment").is_none());
        assert!(parent_children.get("Child Site").is_some());
    }


    #[test]
    fn cross_site_move_anchors_under_peer_attachment_not_child_owned_attachment() {
        let mut config = Config::default();
        config.uisp_integration.enable_uisp = true;
        let canonical = json!({
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

        let mut move_attachment = sample_attachment_option("beta-alpha-60", "Beta - Alpha 60");
        move_attachment.peer_attachment_id = Some("alpha-beta-60".to_string());
        move_attachment.peer_attachment_name = Some("Alpha-Beta-60".to_string());
        move_attachment.download_bandwidth_mbps = Some(940);
        move_attachment.upload_bandwidth_mbps = Some(940);
        move_attachment.capacity_mbps = Some(940);

        let moved = apply_effective_topology_to_network_json(
            &config,
            &canonical,
            &TopologyEditorStateFile {
                schema_version: 1,
                source: "uisp/full2".to_string(),
                generated_unix: None,
                ingress_identity: None,
                nodes: vec![TopologyEditorNode {
                    node_id: "site-beta".to_string(),
                    node_name: "Site Beta".to_string(),
                    latitude: None,
                    longitude: None,
                    current_parent_node_id: Some("site-alpha".to_string()),
                    current_parent_node_name: Some("Site Alpha".to_string()),
                    current_attachment_id: Some("beta-alpha-60".to_string()),
                    current_attachment_name: Some("Beta - Alpha 60".to_string()),
                    can_move: true,
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "site-alpha".to_string(),
                        parent_node_name: "Site Alpha".to_string(),
                        attachment_options: vec![move_attachment],
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
                    node_id: "site-beta".to_string(),
                    logical_parent_node_id: "site-alpha".to_string(),
                    preferred_attachment_id: Some("beta-alpha-60".to_string()),
                    effective_attachment_id: Some("beta-alpha-60".to_string()),
                    fallback_reason: None,
                    all_attachments_suppressed: false,
                    attachments: vec![TopologyEffectiveAttachmentState {
                        attachment_id: "beta-alpha-60".to_string(),
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

        assert!(moved.get("Site Beta").is_none());
        let matt_children = moved["Site Alpha"]["children"]
            .as_object()
            .expect("Site Alpha should keep children");
        let beta_site = matt_children
            .get("Site Beta")
            .and_then(Value::as_object)
            .expect("Site Beta should remain visible under Site Alpha after squashing");
        assert_eq!(beta_site["id"].as_str(), Some("site-beta"));
        assert_eq!(beta_site["parent_site"].as_str(), Some("Site Alpha"));
        assert_eq!(
            beta_site["active_attachment_name"].as_str(),
            Some("Alpha-Beta-60")
        );
        let beta_children = beta_site["children"]
            .as_object()
            .expect("Site Beta subtree should keep its children");
        assert!(beta_children.get("Beta - Alpha 60").is_some());
    }


    #[test]
    fn effective_export_fails_when_reparent_target_parent_is_missing() {
        let canonical = json!({
            "Old Parent": {
                "children": {
                    "Child Site": {
                        "children": {},
                        "id": "child-site",
                        "name": "Child Site",
                        "parent_site": "Old Parent",
                        "type": "Site"
                    }
                },
                "id": "old-parent",
                "name": "Old Parent",
                "type": "Site"
            }
        });
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "test".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "child-site".to_string(),
                node_name: "Child Site".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("old-parent".to_string()),
                current_parent_node_name: Some("Old Parent".to_string()),
                current_attachment_id: None,
                current_attachment_name: None,
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "missing-parent".to_string(),
                    parent_node_name: "Missing Parent".to_string(),
                    attachment_options: Vec::new(),
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
                logical_parent_node_id: "missing-parent".to_string(),
                preferred_attachment_id: None,
                effective_attachment_id: None,
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: Vec::new(),
            }],
        };

        let errors = try_apply_effective_topology_to_network_json(
            &Config::default(),
            &canonical,
            &ui_state,
            &effective,
        )
        .expect_err("missing target parent should fail export");
        let error_text = errors.join(" | ");
        assert!(error_text.contains("Child Site"));
        assert!(error_text.contains("missing-parent"));
    }


    #[test]
    fn effective_export_fails_when_reparent_would_overwrite_child_key() {
        let canonical = json!({
            "Old Parent": {
                "children": {
                    "Child Site": {
                        "children": {},
                        "id": "child-site",
                        "name": "Child Site",
                        "parent_site": "Old Parent",
                        "type": "Site"
                    }
                },
                "id": "old-parent",
                "name": "Old Parent",
                "type": "Site"
            },
            "Target Parent": {
                "children": {
                    "Child Site": {
                        "children": {},
                        "id": "existing-child",
                        "name": "Child Site",
                        "parent_site": "Target Parent",
                        "type": "Site"
                    }
                },
                "id": "target-parent",
                "name": "Target Parent",
                "type": "Site"
            }
        });
        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "test".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "child-site".to_string(),
                node_name: "Child Site".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("old-parent".to_string()),
                current_parent_node_name: Some("Old Parent".to_string()),
                current_attachment_id: None,
                current_attachment_name: None,
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "target-parent".to_string(),
                    parent_node_name: "Target Parent".to_string(),
                    attachment_options: Vec::new(),
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
                logical_parent_node_id: "target-parent".to_string(),
                preferred_attachment_id: None,
                effective_attachment_id: None,
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: Vec::new(),
            }],
        };

        let errors = try_apply_effective_topology_to_network_json(
            &Config::default(),
            &canonical,
            &ui_state,
            &effective,
        )
        .expect_err("child-key collision should fail export");
        let error_text = errors.join(" | ");
        assert!(error_text.contains("Child Site"));
        assert!(error_text.contains("target-parent"));
        assert!(error_text.contains("child key already exists"));
    }


    #[test]
    fn site_reparenting_does_not_create_self_anchored_duplicate_site() {
        let canonical = json!({
            "David Spence": {
                "children": {
                    "Howard Loewen": {
                        "children": {
                            "Howard AP": {
                                "children": {},
                                "downloadBandwidthMbps": 300,
                                "id": "howard-ap",
                                "name": "Howard AP",
                                "parent_site": "Howard Loewen",
                                "type": "AP",
                                "uploadBandwidthMbps": 300
                            }
                        },
                        "downloadBandwidthMbps": 500,
                        "id": "howard-site",
                        "name": "Howard Loewen",
                        "parent_site": "David Spence",
                        "type": "Site",
                        "uploadBandwidthMbps": 500
                    }
                },
                "downloadBandwidthMbps": 800,
                "id": "david-site",
                "name": "David Spence",
                "type": "Site",
                "uploadBandwidthMbps": 800
            }
        });

        let ui_state = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![TopologyEditorNode {
                node_id: "howard-site".to_string(),
                node_name: "Howard Loewen".to_string(),
                latitude: None,
                longitude: None,
                current_parent_node_id: Some("david-site".to_string()),
                current_parent_node_name: Some("David Spence".to_string()),
                current_attachment_id: Some("howard-site".to_string()),
                current_attachment_name: Some("Howard Loewen".to_string()),
                can_move: true,
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "david-site".to_string(),
                    parent_node_name: "David Spence".to_string(),
                    attachment_options: vec![TopologyAttachmentOption {
                        attachment_id: "howard-site".to_string(),
                        attachment_name: "Howard Loewen".to_string(),
                        attachment_kind: "site".to_string(),
                        attachment_role: TopologyAttachmentRole::Unknown,
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
                        health_status: TopologyAttachmentHealthStatus::Disabled,
                        health_reason: None,
                        suppressed_until_unix: None,
                        effective_selected: false,
                    }],
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
                node_id: "howard-site".to_string(),
                logical_parent_node_id: "david-site".to_string(),
                preferred_attachment_id: Some("howard-site".to_string()),
                effective_attachment_id: Some("howard-site".to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![],
            }],
        };

        let moved = apply_effective_topology_to_network_json(
            &Config::default(),
            &canonical,
            &ui_state,
            &effective,
        );

        let david_children = moved["David Spence"]["children"]
            .as_object()
            .expect("David Spence should keep children");
        let howard = david_children
            .get("Howard Loewen")
            .and_then(Value::as_object)
            .expect("Howard Loewen should remain a direct child of David Spence");
        assert_eq!(howard["id"].as_str(), Some("howard-site"));
        let howard_children = howard["children"]
            .as_object()
            .expect("Howard Loewen should keep its children");
        assert!(howard_children.get("Howard Loewen").is_none());
        assert!(howard_children.get("Howard AP").is_some());
        validate_effective_topology_network(
            &Config::default(),
            &canonical,
            &ui_state,
            &effective,
            &moved,
        )
        .expect("self-anchored site attachment should not duplicate the site in the export");
    }
