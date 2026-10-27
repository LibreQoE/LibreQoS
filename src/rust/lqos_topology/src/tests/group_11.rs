    // Part 5 of the attachment-rate override (WO §5/§6): an explicit override
    // must ALSO substitute the ACTIVE attachment's AP-hop (radio) NODE rate, so
    // the runtime squash's `min_attachment_bandwidth` consumes the corrected
    // value. The override WINS over the UISP telemetry, NOT over the squash;
    // an override is inert while an alternate attachment is active.

    fn override_attachment(
        attachment_id: &str,
        attachment_name: &str,
        pair_id: &str,
        download: u64,
        upload: u64,
    ) -> TopologyAttachmentOption {
        let mut option = sample_attachment_option(attachment_id, attachment_name);
        option.pair_id = Some(pair_id.to_string());
        option.download_bandwidth_mbps = Some(download);
        option.upload_bandwidth_mbps = Some(upload);
        option.capacity_mbps = Some(download.min(upload));
        option.has_rate_override = true;
        option
    }

    fn plain_attachment(
        attachment_id: &str,
        attachment_name: &str,
        pair_id: &str,
        download: u64,
        upload: u64,
    ) -> TopologyAttachmentOption {
        let mut option = sample_attachment_option(attachment_id, attachment_name);
        option.pair_id = Some(pair_id.to_string());
        option.download_bandwidth_mbps = Some(download);
        option.upload_bandwidth_mbps = Some(upload);
        option.capacity_mbps = Some(download.min(upload));
        option
    }

    fn override_test_editor_node(
        attachment_options: Vec<TopologyAttachmentOption>,
        current_attachment_id: &str,
        current_attachment_name: &str,
    ) -> TopologyEditorNode {
        TopologyEditorNode {
            node_id: "ladobe".to_string(),
            node_name: "LaDobe".to_string(),
            latitude: None,
            longitude: None,
            current_parent_node_id: Some("monte".to_string()),
            current_parent_node_name: Some("Monte Del Sol".to_string()),
            current_attachment_id: Some(current_attachment_id.to_string()),
            current_attachment_name: Some(current_attachment_name.to_string()),
            can_move: true,
            allowed_parents: vec![TopologyAllowedParent {
                parent_node_id: "monte".to_string(),
                parent_node_name: "Monte Del Sol".to_string(),
                attachment_options,
                all_attachments_suppressed: false,
                has_probe_unavailable_attachments: false,
            }],
            preferred_attachment_id: None,
            preferred_attachment_name: None,
            effective_attachment_id: None,
            effective_attachment_name: None,
        }
    }

    fn override_test_editor_state(
        attachment_options: Vec<TopologyAttachmentOption>,
        active_attachment_id: &str,
    ) -> TopologyEditorStateFile {
        TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![override_test_editor_node(
                attachment_options,
                active_attachment_id,
                active_attachment_id,
            )],
        }
    }

    fn override_test_effective_state(
        active_attachment_id: &str,
    ) -> TopologyEffectiveStateFile {
        TopologyEffectiveStateFile {
            schema_version: 1,
            generated_unix: None,
            canonical_generated_unix: None,
            health_generated_unix: None,
            nodes: vec![TopologyEffectiveNodeState {
                node_id: "ladobe".to_string(),
                logical_parent_node_id: "monte".to_string(),
                preferred_attachment_id: Some(active_attachment_id.to_string()),
                effective_attachment_id: Some(active_attachment_id.to_string()),
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: vec![TopologyEffectiveAttachmentState {
                    attachment_id: active_attachment_id.to_string(),
                    health_status: TopologyAttachmentHealthStatus::Healthy,
                    health_reason: None,
                    suppressed_until_unix: None,
                    probe_enabled: false,
                    probeable: false,
                    effective_selected: true,
                }],
            }],
        }
    }

    /// Writes an operator `AdjustSiteSpeed` layer, mirroring LaDobe's own
    /// `750/750` site speed (the pass that raises the site ABOVE the override
    /// before the squash, which is what makes the lowering reach load-bearing).
    fn write_site_speed_layer(config: &Config, node_id: &str, download: f32, upload: f32) {
        let path = PathBuf::from(&config.lqos_directory).join("lqos_overrides.json");
        fs::write(
            path,
            serde_json::to_string_pretty(&json!({
                "persistent_devices": [],
                "circuit_adjustments": [],
                "network_adjustments": [{
                    "type": "adjust_site_speed",
                    "node_id": node_id,
                    "site_name": node_id,
                    "download_bandwidth_mbps": download,
                    "upload_bandwidth_mbps": upload
                }]
            }))
            .expect("operator layer should serialize"),
        )
        .expect("operator layer should write");
    }

    /// A production-shaped canonical state with `Monte Del Sol -> EPMPF400C (AP,
    /// with radio telemetry) -> LaDobe`. The radio hop is a real canonical node
    /// so `recompile_effective_network_bandwidths` resets it to the telemetry —
    /// exactly why the option-level parts 1-4 cannot reach the squash.
    fn single_hop_canonical(
        hop_download: u64,
        hop_upload: u64,
        options: Vec<TopologyAttachmentOption>,
        active_attachment_id: &str,
    ) -> TopologyCanonicalStateFile {
        let site_node =
            override_test_editor_node(options, active_attachment_id, active_attachment_id);
        let canonical_node = |node_id: &str,
                              node_name: &str,
                              node_kind: &str,
                              download: u64,
                              upload: u64,
                              parent: Option<&str>,
                              allowed_parents: Vec<TopologyAllowedParent>|
         -> TopologyCanonicalNode {
            TopologyCanonicalNode {
                node_id: node_id.to_string(),
                node_name: node_name.to_string(),
                node_kind: node_kind.to_string(),
                current_parent_node_id: parent.map(str::to_string),
                current_parent_node_name: parent.map(|_| "Monte Del Sol".to_string()),
                allowed_parents,
                rate_input: TopologyCanonicalRateInput {
                    intrinsic_download_mbps: Some(download),
                    intrinsic_upload_mbps: Some(upload),
                    legacy_imported_download_mbps: Some(download),
                    legacy_imported_upload_mbps: Some(upload),
                    source: TopologyCanonicalRateInputSource::CompatibilityExport,
                },
                ..TopologyCanonicalNode::default()
            }
        };
        TopologyCanonicalStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            ingress_kind: TopologyCanonicalIngressKind::NativeIntegration,
            nodes: vec![
                canonical_node("monte", "Monte Del Sol", "Site", 8000, 8000, None, vec![]),
                canonical_node(
                    "epmpf400c",
                    "EPMPF400C_LaDobeToMonte",
                    "AP",
                    hop_download,
                    hop_upload,
                    Some("monte"),
                    vec![],
                ),
                canonical_node(
                    "ladobe",
                    "LaDobe",
                    "Site",
                    750,
                    750,
                    Some("epmpf400c"),
                    site_node.allowed_parents.clone(),
                ),
            ],
            compatibility_network_json: json!({
                "Monte Del Sol": {
                    "id": "monte",
                    "name": "Monte Del Sol",
                    "type": "Site",
                    "downloadBandwidthMbps": 8000,
                    "uploadBandwidthMbps": 8000,
                    "children": {
                        "EPMPF400C_LaDobeToMonte": {
                            "id": "epmpf400c",
                            "name": "EPMPF400C_LaDobeToMonte",
                            "type": "AP",
                            "downloadBandwidthMbps": hop_download,
                            "uploadBandwidthMbps": hop_upload,
                            "children": {
                                "LaDobe": {
                                    "id": "ladobe",
                                    "name": "LaDobe",
                                    "type": "Site",
                                    "parent_site": "EPMPF400C_LaDobeToMonte",
                                    "downloadBandwidthMbps": 750,
                                    "uploadBandwidthMbps": 750,
                                    "children": {}
                                }
                            }
                        }
                    }
                }
            }),
            ..TopologyCanonicalStateFile::default()
        }
    }

    fn override_opts(download: u64, upload: u64) -> Vec<TopologyAttachmentOption> {
        vec![override_attachment(
            "epmpf400c",
            "EPMPF400C_LaDobeToMonte",
            "pair-epmpf",
            download,
            upload,
        )]
    }

    // Parts 1-4 WIN (WO §3): a DynamicIntegration attachment with an operator
    // override applies it for ANY non-AUTO attachment; a CLEAR restores the
    // telemetry. This is the option-level reach that Part 5's node fold composes
    // with.
    #[test]
    fn attachment_rate_override_applies_for_dynamic_attachment() {
        let mut option = sample_attachment_option("epmpf400c", "EPMPF400C_LaDobeToMonte");
        option.rate_source = TopologyAttachmentRateSource::DynamicIntegration;
        option.can_override_rate = false;
        option.download_bandwidth_mbps = Some(134);
        option.upload_bandwidth_mbps = Some(50);
        option.capacity_mbps = Some(50);

        let editor = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![override_test_editor_node(
                vec![option],
                "epmpf400c",
                "EPMPF400C_LaDobeToMonte",
            )],
        };

        let mut overrides = TopologyOverridesFile::default();
        assert!(overrides.set_attachment_rate_override_return_changed(
            "ladobe".to_string(),
            "monte".to_string(),
            "epmpf400c".to_string(),
            200,
            100,
        ));
        let prepared = super::apply_attachment_rate_overrides(&editor, &overrides);
        let applied = &prepared.nodes[0].allowed_parents[0].attachment_options[0];
        assert_eq!(applied.download_bandwidth_mbps, Some(200), "the override WINS");
        assert_eq!(applied.upload_bandwidth_mbps, Some(100));
        assert!(applied.has_rate_override);

        // AUTO is never overridable.
        let auto = auto_attachment_option();
        let auto_editor = TopologyEditorStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            generated_unix: None,
            ingress_identity: None,
            nodes: vec![override_test_editor_node(vec![auto.clone()], "auto", "auto")],
        };
        let mut auto_overrides = TopologyOverridesFile::default();
        assert!(auto_overrides.set_attachment_rate_override_return_changed(
            "ladobe".to_string(),
            "monte".to_string(),
            lqos_config::TOPOLOGY_ATTACHMENT_AUTO_ID.to_string(),
            200,
            100,
        ));
        let auto_prepared = super::apply_attachment_rate_overrides(&auto_editor, &auto_overrides);
        let auto_applied = &auto_prepared.nodes[0].allowed_parents[0].attachment_options[0];
        assert_eq!(auto_applied.download_bandwidth_mbps, auto.download_bandwidth_mbps);
        assert!(!auto_applied.has_rate_override, "AUTO must stay non-overridable");

        // A CLEAR restores the telemetry (no override entry).
        let cleared = TopologyOverridesFile::default();
        let restored = super::apply_attachment_rate_overrides(&editor, &cleared);
        let reverted = &restored.nodes[0].allowed_parents[0].attachment_options[0];
        assert_eq!(reverted.download_bandwidth_mbps, Some(134), "a CLEAR restores");
        assert_eq!(reverted.upload_bandwidth_mbps, Some(50));
        assert!(!reverted.has_rate_override);
    }

    // Fixture C target (WO §6): the reach targets the ACTIVE attachment's id
    // (the radio node), never the option/site, and is inert for an inactive or
    // AUTO attachment.
    #[test]
    fn attachment_rate_override_targets_only_the_active_attachment() {
        let active = override_test_editor_state(override_opts(200, 100), "epmpf400c");
        let effective = override_test_effective_state("epmpf400c");
        let targets = super::active_attachment_rate_overrides(&active, &effective);
        assert_eq!(targets.get("epmpf400c"), Some(&(200, 100)));

        // An alternate is active: the override is not targetable.
        let inactive = override_test_editor_state(override_opts(200, 100), "af60");
        let inactive_effective = override_test_effective_state("af60");
        assert!(
            super::active_attachment_rate_overrides(&inactive, &inactive_effective).is_empty(),
            "an inactive attachment override must not target a node"
        );

        // AUTO is never overridable.
        let mut auto = auto_attachment_option();
        auto.has_rate_override = true;
        auto.attachment_id = lqos_config::TOPOLOGY_ATTACHMENT_AUTO_ID.to_string();
        let auto_ui = override_test_editor_state(vec![auto], lqos_config::TOPOLOGY_ATTACHMENT_AUTO_ID);
        let auto_effective =
            override_test_effective_state(lqos_config::TOPOLOGY_ATTACHMENT_AUTO_ID);
        assert!(
            super::active_attachment_rate_overrides(&auto_ui, &auto_effective).is_empty(),
            "the AUTO attachment must never target a node"
        );
    }

    // Fixture C (WO §6): the reach substitutes the ACTIVE attachment's AP-hop
    // (radio) NODE rate at recompile time — the value the runtime squash later
    // consumes — and the descendant site inherits it.
    #[test]
    fn attachment_rate_override_writes_active_ap_hop_node_rate() {
        let canonical = single_hop_canonical(1000, 1000, override_opts(200, 100), "epmpf400c");
        let ui_state = override_test_editor_state(override_opts(200, 100), "epmpf400c");
        let effective = override_test_effective_state("epmpf400c");

        let mut tree = canonical.compatibility_network_json().clone();
        let root = tree.as_object_mut().expect("tree object");
        super::recompile_effective_network_bandwidths(root, &canonical, &ui_state, &effective);

        let hop = tree["Monte Del Sol"]["children"]["EPMPF400C_LaDobeToMonte"]
            .as_object()
            .expect("hop node");
        assert_eq!(
            hop["downloadBandwidthMbps"].as_u64(),
            Some(200),
            "the radio NODE (the squash input), not the option/site, carries the override"
        );
        assert_eq!(hop["uploadBandwidthMbps"].as_u64(), Some(100));
        let site = hop["children"]["LaDobe"].as_object().expect("site node");
        assert_eq!(
            site["downloadBandwidthMbps"].as_u64(),
            Some(200),
            "the corrected hop must reach the descendant site's inherited cap"
        );
    }

    // Fixture A (WO §6): EPMPF400C active + AF60LR down + 200/100 => the site
    // resolves to 200/100. The site's own 750 site-speed raises it past the
    // option cap, so only the reach substitution can hold it at 200/100.
    #[test]
    fn attachment_rate_override_reaches_effective_site_rate() {
        let mut config = test_config();
        config.uisp_integration.enable_uisp = true;
        write_site_speed_layer(&config, "ladobe", 750.0, 750.0);

        let ui_state = override_test_editor_state(override_opts(200, 100), "epmpf400c");
        let effective = override_test_effective_state("epmpf400c");
        let canonical = single_hop_canonical(1000, 1000, override_opts(200, 100), "epmpf400c");

        let squashed = apply_effective_topology_to_canonical_state(
            &config,
            &canonical,
            &ui_state,
            &effective,
            &DirectCircuitHosts::default(),
        );

        let site = squashed["Monte Del Sol"]["children"]["LaDobe"]
            .as_object()
            .expect("site should be squashed under the parent");
        assert_eq!(
            site["downloadBandwidthMbps"].as_u64(),
            Some(200),
            "the active attachment override must WIN over the radio telemetry"
        );
        assert_eq!(site["uploadBandwidthMbps"].as_u64(), Some(100));
        assert_eq!(
            site["active_attachment_name"].as_str(),
            Some("EPMPF400C_LaDobeToMonte")
        );
    }

    // The TrailerCity direction: the radio telemetry is BELOW the operator's
    // rate, so the override must RAISE the site (the squash is a min, so the
    // corrected hop node has to reach the recompiled site value too).
    #[test]
    fn attachment_rate_override_raises_site_to_operator_rate() {
        let mut config = test_config();
        config.uisp_integration.enable_uisp = true;

        let ui_state = override_test_editor_state(override_opts(600, 150), "epmpf400c");
        let effective = override_test_effective_state("epmpf400c");
        let canonical = single_hop_canonical(134, 50, override_opts(600, 150), "epmpf400c");

        let squashed = apply_effective_topology_to_canonical_state(
            &config,
            &canonical,
            &ui_state,
            &effective,
            &DirectCircuitHosts::default(),
        );

        let site = squashed["Monte Del Sol"]["children"]["LaDobe"]
            .as_object()
            .expect("site should be squashed under the parent");
        assert_eq!(
            site["downloadBandwidthMbps"].as_u64(),
            Some(600),
            "the override must RAISE the site above the stale radio telemetry"
        );
        assert_eq!(site["uploadBandwidthMbps"].as_u64(), Some(150));
    }

    // Fixture B (WO §6): AF60LR active + EPMPF400C down + the SAME override =>
    // the site takes AF60LR's rate, NOT 200/100 (the override is a no-leak).
    #[test]
    fn attachment_rate_override_is_inert_when_alternate_attachment_active() {
        let mut config = test_config();
        config.uisp_integration.enable_uisp = true;
        write_site_speed_layer(&config, "ladobe", 750.0, 750.0);

        let options = vec![
            override_attachment("epmpf400c", "EPMPF400C_LaDobeToMonte", "pair-epmpf", 200, 100),
            plain_attachment("af60", "AF60LR-LaDobeToMonte", "pair-af60", 300, 300),
        ];
        let ui_state = override_test_editor_state(options.clone(), "af60");
        let effective = override_test_effective_state("af60");
        let site_node = override_test_editor_node(options, "af60", "af60");
        let canonical = TopologyCanonicalStateFile {
            schema_version: 1,
            source: "uisp/full2".to_string(),
            ingress_kind: TopologyCanonicalIngressKind::NativeIntegration,
            nodes: vec![
                TopologyCanonicalNode {
                    node_id: "monte".to_string(),
                    node_name: "Monte Del Sol".to_string(),
                    node_kind: "Site".to_string(),
                    rate_input: TopologyCanonicalRateInput {
                        intrinsic_download_mbps: Some(8000),
                        intrinsic_upload_mbps: Some(8000),
                        ..TopologyCanonicalRateInput::default()
                    },
                    ..TopologyCanonicalNode::default()
                },
                TopologyCanonicalNode {
                    node_id: "epmpf400c".to_string(),
                    node_name: "EPMPF400C_LaDobeToMonte".to_string(),
                    node_kind: "AP".to_string(),
                    current_parent_node_id: Some("monte".to_string()),
                    rate_input: TopologyCanonicalRateInput {
                        intrinsic_download_mbps: Some(1000),
                        intrinsic_upload_mbps: Some(1000),
                        ..TopologyCanonicalRateInput::default()
                    },
                    ..TopologyCanonicalNode::default()
                },
                TopologyCanonicalNode {
                    node_id: "af60".to_string(),
                    node_name: "AF60LR-LaDobeToMonte".to_string(),
                    node_kind: "AP".to_string(),
                    current_parent_node_id: Some("monte".to_string()),
                    rate_input: TopologyCanonicalRateInput {
                        intrinsic_download_mbps: Some(300),
                        intrinsic_upload_mbps: Some(300),
                        ..TopologyCanonicalRateInput::default()
                    },
                    ..TopologyCanonicalNode::default()
                },
                TopologyCanonicalNode {
                    node_id: "ladobe".to_string(),
                    node_name: "LaDobe".to_string(),
                    node_kind: "Site".to_string(),
                    current_parent_node_id: Some("af60".to_string()),
                    allowed_parents: site_node.allowed_parents.clone(),
                    rate_input: TopologyCanonicalRateInput {
                        intrinsic_download_mbps: Some(750),
                        intrinsic_upload_mbps: Some(750),
                        ..TopologyCanonicalRateInput::default()
                    },
                    ..TopologyCanonicalNode::default()
                },
            ],
            compatibility_network_json: json!({
                "Monte Del Sol": {
                    "id": "monte",
                    "name": "Monte Del Sol",
                    "type": "Site",
                    "downloadBandwidthMbps": 8000,
                    "uploadBandwidthMbps": 8000,
                    "children": {
                        "EPMPF400C_LaDobeToMonte": {
                            "id": "epmpf400c",
                            "name": "EPMPF400C_LaDobeToMonte",
                            "type": "AP",
                            "downloadBandwidthMbps": 1000,
                            "uploadBandwidthMbps": 1000
                        },
                        "AF60LR-LaDobeToMonte": {
                            "id": "af60",
                            "name": "AF60LR-LaDobeToMonte",
                            "type": "AP",
                            "downloadBandwidthMbps": 300,
                            "uploadBandwidthMbps": 300,
                            "children": {
                                "LaDobe": {
                                    "id": "ladobe",
                                    "name": "LaDobe",
                                    "type": "Site",
                                    "parent_site": "AF60LR-LaDobeToMonte",
                                    "downloadBandwidthMbps": 750,
                                    "uploadBandwidthMbps": 750,
                                    "children": {}
                                }
                            }
                        }
                    }
                }
            }),
            ..TopologyCanonicalStateFile::default()
        };

        let squashed = apply_effective_topology_to_canonical_state(
            &config,
            &canonical,
            &ui_state,
            &effective,
            &DirectCircuitHosts::default(),
        );
        let children = squashed["Monte Del Sol"]["children"]
            .as_object()
            .expect("parent keeps children");
        assert!(
            children.get("EPMPF400C_LaDobeToMonte").is_none(),
            "the inactive backhaul stub must be pruned"
        );
        let site = children["LaDobe"]
            .as_object()
            .expect("site should be squashed under the active hop");
        assert_eq!(
            site["downloadBandwidthMbps"].as_u64(),
            Some(300),
            "the inactive EPMPF400C override must NOT leak; the site takes AF60LR's rate"
        );
        assert_eq!(site["uploadBandwidthMbps"].as_u64(), Some(300));
    }

    // E1 (WO §6): clearing the override restores the telemetry rate; a re-apply
    // is idempotent (E3).
    #[test]
    fn attachment_rate_override_clear_restores_telemetry_rate() {
        let mut config = test_config();
        config.uisp_integration.enable_uisp = true;
        write_site_speed_layer(&config, "ladobe", 750.0, 750.0);

        let effective = override_test_effective_state("epmpf400c");
        let canonical = single_hop_canonical(1000, 1000, override_opts(200, 100), "epmpf400c");

        let with_override = override_test_editor_state(override_opts(200, 100), "epmpf400c");
        let capped = apply_effective_topology_to_canonical_state(
            &config,
            &canonical,
            &with_override,
            &effective,
            &DirectCircuitHosts::default(),
        );
        assert_eq!(
            capped["Monte Del Sol"]["children"]["LaDobe"]["downloadBandwidthMbps"].as_u64(),
            Some(200),
            "the override must cap the site"
        );
        // E3: a re-apply is idempotent.
        let reapplied = apply_effective_topology_to_canonical_state(
            &config,
            &canonical,
            &with_override,
            &effective,
            &DirectCircuitHosts::default(),
        );
        assert_eq!(capped, reapplied, "a re-apply must be idempotent");

        // CLEAR: the option returns to the telemetry rate (no override marker).
        let cleared = override_test_editor_state(
            vec![plain_attachment(
                "epmpf400c",
                "EPMPF400C_LaDobeToMonte",
                "pair-epmpf",
                8000,
                8000,
            )],
            "epmpf400c",
        );
        let restored = apply_effective_topology_to_canonical_state(
            &config,
            &canonical,
            &cleared,
            &effective,
            &DirectCircuitHosts::default(),
        );
        let site = restored["Monte Del Sol"]["children"]["LaDobe"]
            .as_object()
            .expect("site node");
        assert_eq!(
            site["downloadBandwidthMbps"].as_u64(),
            Some(750),
            "a cleared override must restore the telemetry rate"
        );
        assert_eq!(site["uploadBandwidthMbps"].as_u64(), Some(750));
    }
