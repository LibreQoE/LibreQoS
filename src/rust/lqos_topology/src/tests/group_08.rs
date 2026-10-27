use lqos_config::{TopologyShapingCircuitInput, TopologyShapingDeviceInput};

// T5 rung 1 gates (t5_topology_manager_consolidation_derivation.md §2):
// E2 structure pins, the two-projection pin, validate()-at-publish, the
// status fold, and the kill-list (the interchange dies; the projections
// carry the world).

#[test]
fn e2_byte_identical_reemit_for_unchanged_input() {
    let config = flat_test_config("lqos-topology-e2-determinism", 0);
    let canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({
        "s10-a": {
            "id": "libreqos:legacy-network-json:node:fc12ff8270a70366",
            "type": "Site",
            "downloadBandwidthMbps": 10_000u64,
            "uploadBandwidthMbps": 10_000u64,
            "children": {
                "s10-a-ap0": {
                    "id": "libreqos:legacy-network-json:node:c99483a36594a604",
                    "type": "ap",
                    "children": {}
                }
            }
        }
    }));
    let view_a = crate::build_operator_view(&canonical, None, None, None, "gen-1")
        .expect("operator view should build");
    let view_b = crate::build_operator_view(&canonical, None, None, None, "gen-1")
        .expect("operator view should build");
    let bytes_a = serde_json::to_string_pretty(&view_a).expect("e2 serializes");
    let bytes_b = serde_json::to_string_pretty(&view_b).expect("e2 serializes");
    assert_eq!(bytes_a, bytes_b, "same input ⇒ byte-identical re-emit");
    let _ = &config;
}

#[test]
fn e2_two_projection_pin_empty_sites_present_in_e2_absent_in_e1() {
    let config = flat_test_config("lqos-topology-e2-two-projection", 0);
    let canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({
        "s10-a": {
            "id": "libreqos:legacy-network-json:node:fc12ff8270a70366",
            "type": "Site",
            "downloadBandwidthMbps": 10_000u64,
            "uploadBandwidthMbps": 10_000u64,
            "children": {
                "s10-a-ap0": {
                    "id": "libreqos:legacy-network-json:node:c99483a36594a604",
                    "type": "ap",
                    "downloadBandwidthMbps": 10_000u64,
                    "uploadBandwidthMbps": 10_000u64,
                    "children": {}
                },
                "s10-b": {
                    "id": "libreqos:legacy-network-json:node:3a05f3f9c95a92be",
                    "type": "Site",
                    "downloadBandwidthMbps": 10_000u64,
                    "uploadBandwidthMbps": 10_000u64,
                    "children": {}
                }
            }
        }
    }));
    let shaping_inputs = TopologyShapingInputsFile {
        schema_version: 1,
        shaping_generation: String::new(),
        circuits: vec![TopologyShapingCircuitInput {
            circuit_id: "c1".to_string(),
            circuit_name: "Bench Client 1".to_string(),
            effective_parent_node_id: "libreqos:legacy-network-json:node:c99483a36594a604"
                .to_string(),
            effective_parent_node_name: "s10-a-ap0".to_string(),
            logical_parent_node_id: None,
            logical_parent_node_name: None,
            anchor_node_id: None,
            anchor_node_name: None,
            effective_attachment_id: None,
            effective_attachment_name: None,
            download_min_mbps: 50.0,
            upload_min_mbps: 25.0,
            download_max_mbps: 200.0,
            upload_max_mbps: 100.0,
            comment: String::new(),
            sqm_override: None,
            tunnel_identity: None,
            vlan_identity: None,
            mpls_identity: None,
            resolution_source: Default::default(),
            devices: vec![TopologyShapingDeviceInput {
                device_id: "dev1".to_string(),
                device_name: "Bench Client 1".to_string(),
                mac: String::new(),
                ipv4: vec!["100.97.0.1".to_string()],
                ipv6: Vec::new(),
                comment: String::new(),
            }],
        }],
        ..TopologyShapingInputsFile::default()
    };

    // E1: the shaping projection ELIDES the circuit-less s10-b chain
    // (the F-LM subtree rule — the recorded acceptance).
    let effective_network = json!({
        "s10-a": {
            "id": "libreqos:legacy-network-json:node:fc12ff8270a70366",
            "name": "s10-a",
            "type": "Site",
            "downloadBandwidthMbps": 10_000u64,
            "uploadBandwidthMbps": 10_000u64,
            "children": {
                "s10-a-ap0": {
                    "id": "libreqos:legacy-network-json:node:c99483a36594a604",
                    "name": "s10-a-ap0",
                    "type": "ap",
                    "downloadBandwidthMbps": 10_000u64,
                    "uploadBandwidthMbps": 10_000u64,
                    "children": {}
                }
            }
        }
    });
    let transaction = compile_dataplane_transaction(
        &effective_network,
        &shaping_inputs,
    );
    let inner_ids: Vec<&str> = transaction
        .inner_ops
        .iter()
        .map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, .. } => node_id.as_str(),
            _ => "",
        })
        .collect();
    assert!(
        !inner_ids
            .iter()
            .any(|id| id.contains("3a05f3f9")),
        "E1 elides the empty site: {inner_ids:?}"
    );

    // E2: the operator projection KEEPS it (F-LM §8-4 resolved).
    let view = crate::build_operator_view(
        &canonical,
        Some(&shaping_inputs),
        Some(&effective_network),
        None,
        "gen-1",
    )
    .expect("operator view should build");
    assert!(
        view.nodes.iter().any(|node| node.id.contains("3a05f3f9") && node.name == "s10-b"),
        "E2 keeps the circuit-less site: {:?}",
        view.nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>()
    );
    // Circuit identity is inline on the circuit node (F10).
    let circuit = view.nodes.iter().find(|node| node.id == "c1").expect("circuit node");
    assert_eq!(circuit.devices.len(), 1);
    assert_eq!(circuit.devices[0].ipv4, vec!["100.97.0.1".to_string()]);
    let _ = &config;
}

#[test]
fn e2_circuit_node_carries_tunnel_identity() {
    // T-arc §2.8: E2 displays the identity on circuit nodes; a circuit
    // without one must be byte-identical to the pre-T-arc schema.
    let canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({}));
    let shaping_inputs: TopologyShapingInputsFile = serde_json::from_value(json!({
        "schema_version": 1,
        "circuits": [
            {
                "circuit_id": "c-tunnel", "circuit_name": "Tunnel Circuit",
                "effective_parent_node_name": "site-1",
                "effective_parent_node_id": "site-1",
                "download_min_mbps": 10.0, "upload_min_mbps": 5.0,
                "download_max_mbps": 100.0, "upload_max_mbps": 50.0,
                "tunnel_identity": { "vni": 100, "vlan": 64 },
                "devices": []
            },
            {
                "circuit_id": "c-plain", "circuit_name": "Plain Circuit",
                "effective_parent_node_name": "site-1",
                "effective_parent_node_id": "site-1",
                "download_min_mbps": 10.0, "upload_min_mbps": 5.0,
                "download_max_mbps": 100.0, "upload_max_mbps": 50.0,
                "devices": []
            }
        ]
    }))
    .expect("shaping inputs should deserialize");
    let view = crate::build_operator_view(&canonical, Some(&shaping_inputs), None, None, "gen-tunnel")
        .expect("operator view should build");

    let tunnel = view.node_by_id("c-tunnel").expect("tunnel circuit node");
    assert_eq!(
        tunnel
            .tunnel_identity
            .map(|identity| (identity.vni, identity.vlan)),
        Some((100, 64)),
        "E2 must display the circuit's tunnel identity"
    );
    let tunnel_json = serde_json::to_value(tunnel).expect("node serializes");
    assert_eq!(tunnel_json["tunnel_identity"]["vni"].as_u64(), Some(100));
    assert_eq!(tunnel_json["tunnel_identity"]["vlan"].as_u64(), Some(64));

    let plain = view.node_by_id("c-plain").expect("plain circuit node");
    assert_eq!(plain.tunnel_identity, None);
    let plain_json = serde_json::to_value(plain).expect("node serializes");
    assert!(
        plain_json.get("tunnel_identity").is_none(),
        "an absent tunnel identity must not serialize a key: {plain_json}"
    );
}

#[test]
fn e2_nodes_carry_rate_source_markers_and_suspended_hides_the_floor() {
    // Rung 1.5 §1.5: E2 displays the effective rates + the source marker;
    // a suspended circuit shows the SENTINEL, never the broker floor.
    let canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({}));
    let shaping_inputs: TopologyShapingInputsFile = serde_json::from_value(json!({
        "schema_version": 1,
        "circuits": [
            { "circuit_id": "c-base", "circuit_name": "Base", "effective_parent_node_name": "site-1", "effective_parent_node_id": "site-1",
              "download_min_mbps": 10.0, "upload_min_mbps": 5.0,
              "download_max_mbps": 100.0, "upload_max_mbps": 50.0, "devices": [] },
            { "circuit_id": "c-override", "circuit_name": "Override", "effective_parent_node_name": "site-1", "effective_parent_node_id": "site-1",
              "download_min_mbps": 20.0, "upload_min_mbps": 10.0,
              "download_max_mbps": 200.0, "upload_max_mbps": 100.0, "devices": [] },
            { "circuit_id": "c-suspended", "circuit_name": "Suspended", "effective_parent_node_name": "site-1", "effective_parent_node_id": "site-1",
              "download_min_mbps": 0.01, "upload_min_mbps": 0.01,
              "download_max_mbps": 0.01, "upload_max_mbps": 0.01, "devices": [] }
        ]
    }))
    .expect("shaping inputs should deserialize");

    let mut overrides = lqos_overrides::OverrideFile::default();
    overrides
        .set_circuit_speed_return_changed(
            "c-override",
            lqos_overrides::CircuitSpeedOverride {
                min_download_bandwidth: Some(20.0),
                max_download_bandwidth: Some(200.0),
                min_upload_bandwidth: Some(10.0),
                max_upload_bandwidth: Some(100.0),
            },
        )
        .expect("override should set");
    overrides
        .set_circuit_speed_return_changed(
            "c-suspended",
            lqos_overrides::CircuitSpeedOverride::suspend_floor(),
        )
        .expect("suspend should set");

    let view = crate::build_operator_view(
        &canonical,
        Some(&shaping_inputs),
        None,
        Some(&overrides),
        "gen-markers",
    )
    .expect("operator view should build");

    assert_eq!(
        view.node_by_id("c-base").unwrap().rate_source,
        lqos_config::OperatorRateSource::Base
    );
    assert_eq!(
        view.node_by_id("c-override").unwrap().rate_source,
        lqos_config::OperatorRateSource::Override
    );
    let suspended = view.node_by_id("c-suspended").unwrap();
    assert_eq!(
        suspended.rate_source,
        lqos_config::OperatorRateSource::SuspendSentinel
    );
    assert_eq!(
        suspended.download_mbps, None,
        "a suspended circuit must not display a numeric floor"
    );
    assert_eq!(suspended.upload_mbps, None);
    let json = serde_json::to_value(suspended).expect("node serializes");
    assert_eq!(json["rate_source"].as_str(), Some("suspend-sentinel"));
    // Byte-stability: the base marker is omitted.
    let base_json =
        serde_json::to_value(view.node_by_id("c-base").unwrap()).expect("node serializes");
    assert!(base_json.get("rate_source").is_none());
}

#[test]
fn publish_rejects_an_invalid_model_loudly() {
    let lqos_directory = unique_temp_dir("lqos-topology-e2-invalid-model");
    let config = Config {
        lqos_directory: lqos_directory.to_string_lossy().to_string(),
        state_directory: None,
        ..Config::default()
    };
    // A CYCLIC model — ruling 6: publish must LOUD-reject, never
    // silently pass through.
    let mut canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({}));
    canonical.nodes.push(TopologyCanonicalNode {
        node_id: "a".to_string(),
        node_name: "A".to_string(),
        node_kind: "Site".to_string(),
        current_parent_node_id: Some("b".to_string()),
        ..TopologyCanonicalNode::default()
    });
    canonical.nodes.push(TopologyCanonicalNode {
        node_id: "b".to_string(),
        node_name: "B".to_string(),
        node_kind: "Site".to_string(),
        current_parent_node_id: Some("a".to_string()),
        ..TopologyCanonicalNode::default()
    });
    let artifacts = EffectiveTopologyArtifacts {
        canonical,
        effective: TopologyEffectiveStateFile::default(),
        ui_state: TopologyEditorStateFile::default(),
        effective_network: None,
    };
    let result = publish_effective_topology_artifacts(&config, &artifacts);
    assert!(result.is_err(), "publish rejects a cyclic model");
    let status = TopologyRuntimeStatusFile::load(&config).expect("status should load");
    assert!(!status.ready, "the error status marks runtime not ready");
    assert!(
        status.error.as_deref().unwrap_or_default().contains("validation"),
        "the error names the validation reject: {:?}",
        status.error
    );
}

#[test]
fn fna_minted_generation_equals_next_published_on_revert_to_base() {
    // F-NA binding gate 1 (docs/false_no_ack_outcome_card.md): the ws
    // mint must use the RUNTIME build variant and equal the generation
    // the publisher ACTUALLY stamps next — pinned on the revert-to-base
    // path (a return to a retained generation; the key-diff blind spot).
    let config = flat_test_config("lqos-topology-fna-mint-revert", 2);
    write_flat_test_devices(&config, &[("c1", 200.0)]);
    let canonical = crate::load_canonical_topology_state(&config);
    let health = TopologyAttachmentHealthStateFile::default();

    fn runtime_publish(config: &Config, canonical: &TopologyCanonicalStateFile,
                       health: &TopologyAttachmentHealthStateFile, source: &str) -> String {
        let overrides = TopologyOverridesFile::load_for_config(config)
            .expect("overrides load");
        let artifacts = crate::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            config, canonical, &overrides, health,
        )
        .expect("runtime build should succeed");
        publish_effective_topology_artifacts(config, &artifacts)
            .expect("publish should succeed");
        TopologyShapingInputsFile::load(config)
            .expect("shaping inputs should load")
            .shaping_generation
    }

    fn write_operator_override(config: &Config, present: bool) {
        let adjustments = if present {
            json!([{
                "type": "circuit_adjust_speed",
                "circuit_id": "c1",
                "min_download_bandwidth": 190.0,
                "max_download_bandwidth": 190.0,
                "min_upload_bandwidth": 95.0,
                "max_upload_bandwidth": 95.0
            }])
        } else {
            json!([])
        };
        fs::write(
            PathBuf::from(&config.lqos_directory).join("lqos_overrides.json"),
            serde_json::to_string_pretty(&json!({
                "persistent_devices": [],
                "circuit_adjustments": adjustments,
                "network_adjustments": []
            }))
            .expect("override json should serialize"),
        )
        .expect("overrides file should write");
    }

    // Round B: the override is ACTIVE (190/95) and published.
    write_operator_override(&config, true);
    let gen_overridden =
        runtime_publish(&config, &canonical, &health, "src-b");

    // REVERT: the override file returns to base. MINT (read-only), then
    // let the publisher run its next round — the two MUST agree.
    write_operator_override(&config, false);
    let minted = crate::mint_next_shaping_generation(&config)
        .expect("mint should succeed")
        .expect("flat world should produce shaping inputs");
    let published_after_revert =
        runtime_publish(&config, &canonical, &health, "src-c");

    assert_eq!(minted, published_after_revert,
        "the mint must equal the next-published generation (binding gate 1)");
    assert_ne!(minted, gen_overridden,
        "the revert genuinely changed the world (base != overridden)");
    // Source-stable generations: the post-revert world re-lands on the
    // BASE generation — publish once more from a pristine override file
    // world to prove the base round-trips to the same hash.
    let _ = fs::remove_file(PathBuf::from(&config.lqos_directory).join("lqos_overrides.json"));
    let base_republish = runtime_publish(&config, &canonical, &health, "src-d");
    assert_eq!(base_republish, published_after_revert,
        "same content ⇒ same generation (the retained-key premise)");
}

#[test]
fn stormguard_site_speed_clamp_changes_the_inner_op_ceil() {
    // K1b §4.1 item 1 (the failing-at-HEAD gate): a stormguard
    // `AdjustSiteSpeed` entry must fold into the effective tree so the
    // compiled site inner-op ceil CHANGES. At HEAD the clamp is inert
    // (a stormguard layer write republishes an identical transaction).
    assert_eq!(
        site_ceil_with_stormguard_layer("clamp", None, false),
        100_000,
        "the stormguard clamp must fold into the site inner-op ceil"
    );
}

#[test]
fn stormguard_name_only_clamp_folds_via_the_object_key() {
    // K1b drill finding: StormGuard writes `node_id: None`, and the effective
    // compatibility tree (legacy network.json) keys nodes by name with NO
    // `name` field — the fold must fall back to the object key or the clamp is
    // inert on the real bridge.
    assert_eq!(
        site_ceil_with_stormguard_layer("nameonly", None, true),
        100_000,
        "a name-only clamp must fold via the effective-tree object key"
    );
}

#[test]
fn stormguard_lapsed_site_speed_clamp_does_not_fold() {
    // K1b §4.6 pin 1 defense-in-depth: a lapsed machine clamp must not apply
    // even before the periodic sweep drops it from the layer file.
    assert_eq!(
        site_ceil_with_stormguard_layer("lapsed", Some(1), false),
        1_000_000,
        "a lapsed clamp must leave the canonical ceil untouched"
    );
}

fn site_ceil_with_stormguard_layer(
    name_suffix: &str,
    expires_at_unix: Option<u64>,
    name_only: bool,
) -> u32 {
    let mut config = tunnel_test_config(&format!("lqos-topology-sg-site-{name_suffix}"));
    config.stormguard = Some(lqos_config::StormguardConfig {
        enabled: true,
        dry_run: false,
        ..Default::default()
    });
    let mut compatibility_site = json!({
        "id": "site-1",
        "type": "Site",
        "downloadBandwidthMbps": 1000,
        "uploadBandwidthMbps": 1000,
        "children": {}
    });
    if !name_only {
        compatibility_site["name"] = json!("site-1");
    }
    let canonical = TopologyCanonicalStateFile {
        ingress_kind: TopologyCanonicalIngressKind::LegacyNetworkJson,
        source: "legacy/network.json".to_string(),
        nodes: vec![TopologyCanonicalNode {
            node_id: "site-1".to_string(),
            node_name: "site-1".to_string(),
            node_kind: "Site".to_string(),
            rate_input: TopologyCanonicalRateInput {
                intrinsic_download_mbps: Some(1000),
                intrinsic_upload_mbps: Some(1000),
                legacy_imported_download_mbps: Some(1000),
                legacy_imported_upload_mbps: Some(1000),
                source: TopologyCanonicalRateInputSource::OperatorOverride,
            },
            ..TopologyCanonicalNode::default()
        }],
        compatibility_network_json: json!({
            "site-1": compatibility_site
        }),
        ..TopologyCanonicalStateFile::default()
    };
    // A base circuit under the site so the site materializes in E1.
    let mut csv = String::from(
        "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
    );
    csv.push_str(
        "\"c1\",\"c1\",\"d1\",\"D1\",\"site-1\",\"site-1\",\"\",\"\",\"192.0.2.1/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
    );
    fs::write(ConfigShapedDevices::path_for_config(&config), csv)
        .expect("ShapedDevices.csv should write");

    let mut layer_entry = json!({
        "type": "adjust_site_speed",
        "site_name": "site-1",
        "download_bandwidth_mbps": 100.0,
        "upload_bandwidth_mbps": 50.0
    });
    if !name_only {
        layer_entry["node_id"] = json!("site-1");
    }
    if let Some(expires) = expires_at_unix {
        layer_entry["expires_at_unix"] = json!(expires);
    }
    fs::write(
        PathBuf::from(&config.lqos_directory).join("lqos_overrides.stormguard.json"),
        serde_json::to_string_pretty(&json!({
            "persistent_devices": [],
            "circuit_adjustments": [],
            "network_adjustments": [layer_entry]
        }))
        .expect("stormguard layer should serialize"),
    )
    .expect("stormguard layer should write");

    let artifacts =
        crate::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            &config,
            &canonical,
            &TopologyOverridesFile::default(),
            &TopologyAttachmentHealthStateFile::default(),
        )
        .expect("runtime build should succeed");
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let network = artifacts
        .effective_network
        .clone()
        .expect("effective network should exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    tx.inner_ops
        .iter()
        .find_map(|op| match op {
            DataplaneInnerOp::Upsert {
                node_id, download, ..
            } if node_id == "site-1" => Some(download.ceil_kbps),
            _ => None,
        })
        .expect("the site must emit an inner op")
}

#[test]
fn stormguard_circuit_aqm_fallback_folds_as_a_post_pass() {
    // K1b ph2 §9.1-3 failing-at-HEAD gate: a stormguard `CircuitAdjustAqm`
    // entry must set the assembled circuit row's sqm_override (a POST-pass,
    // outranking a device-level seed), and the compiled leaf aqm must change.
    use lqos_overrides::CircuitAdjustment;
    let circuit = lqos_config::TopologyShapingCircuitInput {
        circuit_id: "c1".to_string(),
        circuit_name: "c1".to_string(),
        effective_parent_node_id: "site-1".to_string(),
        // A stale device-level seed must be outvoted by the circuit entry.
        sqm_override: Some("none".to_string()),
        ..lqos_config::TopologyShapingCircuitInput::default()
    };

    let mut overrides = lqos_overrides::OverrideFile::default();
    overrides.add_circuit_adjustment(CircuitAdjustment::CircuitAdjustAqm {
        circuit_id: "c1".to_string(),
        aqm_token: "cake diffserv4".to_string(),
        expires_at_unix: None,
    });
    let mut warnings = Vec::new();
    let mut circuits = vec![circuit];
    crate::apply_circuit_aqm_overrides(&mut circuits, &overrides, &mut warnings);
    assert_eq!(circuits[0].sqm_override.as_deref(), Some("cake diffserv4"));
    assert!(warnings.is_empty(), "no warnings in integration mode: {warnings:?}");

    let inputs = TopologyShapingInputsFile {
        schema_version: 1,
        circuits,
        ..Default::default()
    };
    let network = json!({
        "site-1": {
            "id": "site-1",
            "name": "site-1",
            "downloadBandwidthMbps": 1000,
            "uploadBandwidthMbps": 1000,
            "children": {}
        }
    });
    let tx = compile_dataplane_transaction(&network, &inputs);
    let aqm = tx
        .leaf_ops
        .iter()
        .find_map(|op| match op {
            DataplaneLeafOp::Upsert { circuit_id, aqm, .. } if circuit_id == "c1" => {
                Some(aqm.clone())
            }
            _ => None,
        })
        .expect("c1 must emit a leaf");
    assert_eq!(
        aqm,
        DataplaneLeafAqm::Cake {
            diffserv4: true,
            rtt_ms: None
        },
        "the circuit AQM fallback must change the compiled leaf aqm"
    );
}

#[test]
fn stormguard_circuit_aqm_applies_in_both_ingress_modes() {
 // (§9.4): the circuit-AQM post-pass is NOT integration-only —
    // it is a machine-authored runtime fallback, so it applies in legacy-DIY
    // mode too (the ph1 site-clamp precedent). No mode-gated WARN.
    use lqos_overrides::CircuitAdjustment;
    let circuit = lqos_config::TopologyShapingCircuitInput {
        circuit_id: "c1".to_string(),
        sqm_override: None,
        ..lqos_config::TopologyShapingCircuitInput::default()
    };
    let mut overrides = lqos_overrides::OverrideFile::default();
    overrides.add_circuit_adjustment(CircuitAdjustment::CircuitAdjustAqm {
        circuit_id: "c1".to_string(),
        aqm_token: "cake".to_string(),
        expires_at_unix: None,
    });
    let mut warnings = Vec::new();
    let mut circuits = vec![circuit];
    crate::apply_circuit_aqm_overrides(&mut circuits, &overrides, &mut warnings);
    assert_eq!(
        circuits[0].sqm_override.as_deref(),
        Some("cake"),
        "the fallback must apply in every ingress mode"
    );
    assert!(
        !warnings.iter().any(|w| w.contains("legacy_ignored")),
        "the mode-gated WARN is gone: {warnings:?}"
    );
}

#[test]
fn stormguard_circuit_aqm_absent_circuit_warns() {
    use lqos_overrides::CircuitAdjustment;
    let mut overrides = lqos_overrides::OverrideFile::default();
    overrides.add_circuit_adjustment(CircuitAdjustment::CircuitAdjustAqm {
        circuit_id: "ghost".to_string(),
        aqm_token: "cake".to_string(),
        expires_at_unix: None,
    });
    let mut warnings = Vec::new();
    let mut circuits: Vec<lqos_config::TopologyShapingCircuitInput> = Vec::new();
    crate::apply_circuit_aqm_overrides(&mut circuits, &overrides, &mut warnings);
    assert!(
        warnings.iter().any(|w| w.contains("absent")),
        "an absent circuit must WARN loudly: {warnings:?}"
    );
}

#[test]
fn stormguard_lapsed_circuit_aqm_does_not_fold() {
    // §9.2 defense-in-depth: a lapsed machine entry must not apply even before
    // the periodic sweep drops it from the layer file.
    use lqos_overrides::CircuitAdjustment;
    let circuit = lqos_config::TopologyShapingCircuitInput {
        circuit_id: "c1".to_string(),
        sqm_override: Some("none".to_string()),
        ..lqos_config::TopologyShapingCircuitInput::default()
    };
    let mut overrides = lqos_overrides::OverrideFile::default();
    overrides.add_circuit_adjustment(CircuitAdjustment::CircuitAdjustAqm {
        circuit_id: "c1".to_string(),
        aqm_token: "cake".to_string(),
        expires_at_unix: Some(1),
    });
    let mut warnings = Vec::new();
    let mut circuits = vec![circuit];
    crate::apply_circuit_aqm_overrides(&mut circuits, &overrides, &mut warnings);
    assert_eq!(
        circuits[0].sqm_override.as_deref(),
        Some("none"),
        "a lapsed entry must not apply"
    );
}

#[test]
fn dynamic_range_emits_unconfirmed_leaf_and_prefix_map() {
    // Dynamic-ranges H1: a configured range injects at the transaction stage —
    // the unconfirmed leaf + its prefix Map, with the parent site materialized.
    use lqos_config::{DynamicCircuitRangeRule, TopologyShapingInputsFile};
    let network = json!({
        "site-a": {
            "id": "site-a-id",
            "name": "site-a",
            "downloadBandwidthMbps": 1000,
            "uploadBandwidthMbps": 1000,
            "children": {}
        }
    });
    let inputs = TopologyShapingInputsFile {
        schema_version: 1,
        ..Default::default()
    };
    let range = DynamicCircuitRangeRule {
        name: "pool-1".to_string(),
        ip_range: "10.50.0.0/24".parse().expect("cidr"),
        download_min_mbps: 1.0,
        upload_min_mbps: 1.0,
        download_max_mbps: 5.0,
        upload_max_mbps: 5.0,
        attach_to: "site-a".to_string(),
    };
    let tx = crate::compile_dataplane_transaction_with_ranges(&network, &inputs, &[range]);
    assert!(
        tx.leaf_ops.iter().any(|op| matches!(
            op,
            DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "dynamic-range:pool-1"
        )),
        "the range must emit its unconfirmed leaf"
    );
    assert!(
        tx.mapping_ops.iter().any(|op| matches!(
            op,
            DataplaneMappingOp::Map { prefix_len: 24, circuit_id, .. } if circuit_id == "dynamic-range:pool-1"
        )),
        "the range must emit its prefix Map"
    );
    assert!(
        tx.inner_ops.iter().any(|op| matches!(
            op,
            DataplaneInnerOp::Upsert { node_id, .. } if node_id == "site-a-id"
        )),
        "the range leaf's parent site must materialize"
    );
}

#[test]
fn dynamic_range_unresolvable_parent_defers_loudly() {
    // §1.5: an unresolvable parent defers the whole range row (no leaf, no Map).
    use lqos_config::{DynamicCircuitRangeRule, TopologyShapingInputsFile};
    let network = json!({
        "site-a": {
            "id": "site-a-id",
            "name": "site-a",
            "downloadBandwidthMbps": 1000,
            "uploadBandwidthMbps": 1000,
            "children": {}
        }
    });
    let inputs = TopologyShapingInputsFile {
        schema_version: 1,
        ..Default::default()
    };
    let range = DynamicCircuitRangeRule {
        name: "pool-1".to_string(),
        ip_range: "10.50.0.0/24".parse().expect("cidr"),
        download_min_mbps: 1.0,
        upload_min_mbps: 1.0,
        download_max_mbps: 5.0,
        upload_max_mbps: 5.0,
        attach_to: "no-such-site".to_string(),
    };
    let tx = crate::compile_dataplane_transaction_with_ranges(&network, &inputs, &[range]);
    assert!(
        !tx.leaf_ops.iter().any(|op| matches!(
            op,
            DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id.starts_with("dynamic-range:")
        )),
        "unresolvable parent must not emit a leaf"
    );
    assert!(
        tx.mapping_ops.is_empty(),
        "unresolvable parent must not emit a Map"
    );
}

#[test]
fn dynamic_range_blank_or_whitespace_attach_uses_the_implicit_root() {
    // Blank attach_to is the dataplane root (not "no attachment"): the range
    // leaf hangs off the implicit, unclamped root and still emits its Map.
    use lqos_config::{DynamicCircuitRangeRule, TopologyShapingInputsFile};
    let network = json!({});
    let inputs = TopologyShapingInputsFile {
        schema_version: 1,
        ..Default::default()
    };
    let range = |name: &str, cidr: &str, attach_to: &str| DynamicCircuitRangeRule {
        name: name.to_string(),
        ip_range: cidr.parse().expect("cidr"),
        download_min_mbps: 1.0,
        upload_min_mbps: 1.0,
        download_max_mbps: 5.0,
        upload_max_mbps: 5.0,
        attach_to: attach_to.to_string(),
    };
    let tx = crate::compile_dataplane_transaction_with_ranges(
        &network,
        &inputs,
        &[
            range("root-1", "10.50.0.0/24", ""),
            range("root-2", "10.51.0.0/24", "   "),
        ],
    );

    for (circuit_id, network_address) in [
        ("dynamic-range:root-1", "10.50.0.0"),
        ("dynamic-range:root-2", "10.51.0.0"),
    ] {
        let (parent_node_id, download, upload) = tx
            .leaf_ops
            .iter()
            .find_map(|op| match op {
                DataplaneLeafOp::Upsert {
                    circuit_id: id,
                    parent_node_id,
                    download,
                    upload,
                    ..
                } if id == circuit_id => Some((parent_node_id, download, upload)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{circuit_id} must emit a root leaf, not DEFER"));
        assert_eq!(parent_node_id, DATA_PLANE_ROOT_NODE_ID);
        // The root is unclamped: the leaf keeps its configured commit/ceil.
        assert_eq!(download.commit_kbps, 1_000);
        assert_eq!(download.ceil_kbps, 5_000);
        assert_eq!(upload.commit_kbps, 1_000);
        assert_eq!(upload.ceil_kbps, 5_000);

        assert!(
            tx.mapping_ops.iter().any(|op| matches!(
                op,
                DataplaneMappingOp::Map {
                    prefix_len: 24,
                    ip,
                    circuit_id: id,
                    ..
                } if id == circuit_id && ip.to_string() == network_address
            )),
            "{circuit_id} must emit its prefix Map"
        );
    }

    assert!(
        tx.inner_ops.is_empty(),
        "the root is implicit; a root range keeps nothing in circuit_bearing_nodes"
    );
}

#[test]
fn verify_published_shaping_generation_round_trip_and_stale_override() {
    // K1b §4.8 / card v2 oracles: the readiness verifier is the read-only
    // dry-run mint of the CURRENT world compared against the published
    // generation. Round-trip TRUE; a world change without a republish FALSE;
    // deterministic dry-run == stamped generation.
    let config = flat_test_config("lqos-topology-verify-published", 2);
    write_flat_test_devices(&config, &[("c1", 200.0)]);
    let canonical = crate::load_canonical_topology_state(&config);
    let health = TopologyAttachmentHealthStateFile::default();

    // The custody verify compares the SOURCE generation (the world identity),
    // which the publish now recomputes after its artifact writes.
    let publish = |_label: &str| {
        let overrides = TopologyOverridesFile::load_for_config(&config)
            .expect("overrides should load");
        let artifacts =
            crate::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
                &config,
                &canonical,
                &overrides,
                &health,
            )
            .expect("runtime build should succeed");
        crate::publish_effective_topology_artifacts(&config, &artifacts)
            .expect("publish should succeed");
    };

    publish("src-a");
    assert_eq!(
        crate::verify_published_shaping_generation(&config).expect("verify should run").verified,
        Some(true),
        "the published generation must verify against a fresh mint"
    );

    // Determinism: the read-only mint equals the stamped status generation.
    let minted = crate::mint_next_shaping_generation(&config)
        .expect("mint should run")
        .expect("flat world should produce shaping inputs");
    let status = TopologyRuntimeStatusFile::load(&config).expect("status should load");
    assert_eq!(
        minted, status.shaping_generation,
        "dry-run mint == published generation"
    );

    // Stale: change the world (a circuit override) WITHOUT republishing.
    fs::write(
        PathBuf::from(&config.lqos_directory).join("lqos_overrides.json"),
        serde_json::to_string_pretty(&json!({
            "persistent_devices": [],
            "circuit_adjustments": [{
                "type": "circuit_adjust_speed",
                "circuit_id": "c1",
                "min_download_bandwidth": 190.0,
                "max_download_bandwidth": 190.0,
                "min_upload_bandwidth": 95.0,
                "max_upload_bandwidth": 95.0
            }],
            "network_adjustments": []
        }))
        .expect("override json should serialize"),
    )
    .expect("overrides file should write");
    // Age the status OUT of the bounded source-lag tolerance, so the world
    // change is STALE (a FRESH world-ahead is tolerated — see
    // `verify_published_shaping_generation_tolerates_a_fresh_source_lag`).
    let mut status = TopologyRuntimeStatusFile::load(&config).expect("status should load");
    status.generated_unix = Some(0);
    status.save(&config).expect("status should save");
    let stale = crate::verify_published_shaping_generation(&config).expect("verify should run");
    assert_eq!(
        stale.verified,
        Some(false),
        "a STALE world change without a republish must fail the verify"
    );
    assert_eq!(
        stale.failed_leg,
        Some(crate::ShapingCustodyLeg::WorldGenerationMismatch),
        "§0b: the failing custody leg is NAMED (world↔generation)"
    );

    // Republishing the changed world restores the agreement.
    publish("src-b");
    assert_eq!(
        crate::verify_published_shaping_generation(&config).expect("verify should run").verified,
        Some(true),
        "republishing the changed world restores the agreement"
    );
}

#[test]
fn verify_published_shaping_generation_is_read_only() {
 // Card v2: the readiness verifier runs per tick FOREVER and
    // must never write — no override-file create/migrate, no quarantine.
    // NON-flat config: exercises the validation-export override load (a write
    // site found by the #12) — flat mode bypasses it.
    let lqos_directory = unique_temp_dir("lqos-topology-verify-readonly-nonflat");
    let mut config = Config {
        lqos_directory: lqos_directory.to_string_lossy().to_string(),
        state_directory: None,
        ..Config::default()
    };
    config.queues.override_available_queues = Some(2);
    write_flat_test_devices(&config, &[("c1", 200.0)]);
    let overrides_path = PathBuf::from(&config.lqos_directory).join("lqos_overrides.json");
    let stormguard_path =
        PathBuf::from(&config.lqos_directory).join("lqos_overrides.stormguard.json");
    assert!(!overrides_path.exists(), "precondition: no operator overrides file");
    assert!(!stormguard_path.exists(), "precondition: no stormguard overrides file");

    // No publish: the verifier must not fabricate any part of the world it
    // measures (the self-satisfaction class).
    let _ = crate::verify_published_shaping_generation(&config);

    assert!(
        !overrides_path.exists(),
        "verify must not create the operator overrides file"
    );
    assert!(
        !stormguard_path.exists(),
        "verify must not create the stormguard overrides file"
    );
}

/// Leg 1 NEGATIVE (review): a STALE / WRONG `source_generation` in the status
/// must FAIL the custody check. The source generation is the WORLD identity;
/// the one-publish shaping-lag tolerance must not blind the check to a bogus
/// world token while the artifacts are otherwise consistent.
#[test]
fn verify_published_shaping_generation_rejects_a_wrong_source_generation() {
    let config = flat_test_config("lqos-topology-verify-wrong-source", 2);
    write_flat_test_devices(&config, &[("c1", 200.0)]);
    let canonical = crate::load_canonical_topology_state(&config);
    let health = TopologyAttachmentHealthStateFile::default();
    let overrides =
        TopologyOverridesFile::load_for_config(&config).expect("overrides should load");
    let artifacts =
        crate::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            &config,
            &canonical,
            &overrides,
            &health,
        )
        .expect("runtime build should succeed");
    crate::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("publish should succeed");
    assert_eq!(
        crate::verify_published_shaping_generation(&config).expect("verify should run").verified,
        Some(true),
        "the freshly published world verifies"
    );

    // Tamper the status world token AND age it out of the bounded lag
    // tolerance (the artifacts stay consistent) — a GENUINELY stale source.
    let mut status = TopologyRuntimeStatusFile::load(&config).expect("status should load");
    status.source_generation = "stale-source-generation".to_string();
    status.generated_unix = Some(0); // ancient: outside SOURCE_LAG_TOLERANCE_SECS
    status.save(&config).expect("tampered status should save");
    let tampered = crate::verify_published_shaping_generation(&config).expect("verify should run");
    assert_eq!(
        tampered.verified,
        Some(false),
        "a stale/wrong source generation MUST fail the custody check"
    );
    assert_eq!(
        tampered.failed_leg,
        Some(crate::ShapingCustodyLeg::WorldGenerationMismatch),
        "§0b: the failing custody leg is NAMED (world↔generation)"
    );
}

/// Leg 1 POSITIVE (the churn): a FRESH status whose source token differs from
/// the live source (the integration churned between publishes) must still
/// PASS — the world is merely ahead and the runtime will republish, so the
/// tracker must refresh its parent cache rather than freeze it.
#[test]
fn verify_published_shaping_generation_tolerates_a_fresh_source_lag() {
    let config = flat_test_config("lqos-topology-verify-source-lag", 2);
    write_flat_test_devices(&config, &[("c1", 200.0)]);
    let canonical = crate::load_canonical_topology_state(&config);
    let health = TopologyAttachmentHealthStateFile::default();
    let overrides =
        TopologyOverridesFile::load_for_config(&config).expect("overrides should load");
    let artifacts =
        crate::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            &config,
            &canonical,
            &overrides,
            &health,
        )
        .expect("runtime build should succeed");
    crate::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("publish should succeed");

    // A DIFFERENT source token, but the status is FRESH (generated just now —
    // a real clock value, NOT a far-future stamp).
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be sane")
        .as_secs();
    let mut status = TopologyRuntimeStatusFile::load(&config).expect("status should load");
    status.source_generation = "the-next-churn-source".to_string();
    status.generated_unix = Some(now_unix);
    status.save(&config).expect("status should save");
    assert_eq!(
        crate::verify_published_shaping_generation(&config).expect("verify should run").verified,
        Some(true),
        "a fresh source lag must be tolerated (the world is ahead, not stale)"
    );
}

/// Leg 1 NEGATIVE (review): a FAR-FUTURE `generated_unix` must NOT defeat the
/// freshness bound (`saturating_sub` would read age 0) — a wrong source with a
/// future stamp must still FAIL.
#[test]
fn verify_published_shaping_generation_rejects_a_future_stamp_source() {
    let config = flat_test_config("lqos-topology-verify-future-stamp", 2);
    write_flat_test_devices(&config, &[("c1", 200.0)]);
    let canonical = crate::load_canonical_topology_state(&config);
    let health = TopologyAttachmentHealthStateFile::default();
    let overrides =
        TopologyOverridesFile::load_for_config(&config).expect("overrides should load");
    let artifacts =
        crate::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            &config,
            &canonical,
            &overrides,
            &health,
        )
        .expect("runtime build should succeed");
    crate::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("publish should succeed");

    let mut status = TopologyRuntimeStatusFile::load(&config).expect("status should load");
    status.source_generation = "wrong-source-future-stamp".to_string();
    status.generated_unix = Some(u64::MAX); // far future: NOT a valid freshness signal
    status.save(&config).expect("status should save");
    let tampered = crate::verify_published_shaping_generation(&config).expect("verify should run");
    assert_eq!(
        tampered.verified,
        Some(false),
        "a far-future stamp must not defeat the freshness bound"
    );
    assert_eq!(
        tampered.failed_leg,
        Some(crate::ShapingCustodyLeg::WorldGenerationMismatch),
        "§0b: the failing custody leg is NAMED (world↔generation)"
    );
}

/// Leg 2 CHURNED-WORLD TAMPER NEGATIVE (review / DRIVER (B)): a load->edit->save
/// on the artifact-of-record (which preserves `shaping_generation`) during a
/// CHURN must FAIL — the recorded content-only digest catches it even though the
/// world moved (the lag tier is content-exact, not identity-only).
#[test]
fn verify_published_shaping_generation_rejects_a_churned_world_tampered_artifact() {
    let config = flat_test_config("lqos-topology-verify-churn-tamper", 2);
    write_flat_test_devices(&config, &[("c1", 200.0)]);
    let canonical = crate::load_canonical_topology_state(&config);
    let health = TopologyAttachmentHealthStateFile::default();
    let overrides =
        TopologyOverridesFile::load_for_config(&config).expect("overrides should load");
    let artifacts =
        crate::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            &config,
            &canonical,
            &overrides,
            &health,
        )
        .expect("runtime build should succeed");
    crate::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("publish should succeed");

    // Move the WORLD (an override) so the on-disk artifact legitimately lags (the
    // churn), THEN tamper it via load->edit->save — the `shaping_generation`
    // field is preserved (the residual edit->save pattern).
    fs::write(
        PathBuf::from(&config.lqos_directory).join("lqos_overrides.json"),
        serde_json::to_string_pretty(&json!({
            "persistent_devices": [],
            "circuit_adjustments": [{
                "type": "circuit_adjust_speed",
                "circuit_id": "c1",
                "min_download_bandwidth": 190.0,
                "max_download_bandwidth": 190.0,
                "min_upload_bandwidth": 95.0,
                "max_upload_bandwidth": 95.0
            }],
            "network_adjustments": []
        }))
        .expect("override json should serialize"),
    )
    .expect("overrides file should write");
    let mut on_disk =
        TopologyShapingInputsFile::load(&config).expect("on-disk shaping inputs should load");
    on_disk.circuits[0].effective_parent_node_name = "Tampered".to_string();
    on_disk
        .save(&config)
        .expect("tampered shaping inputs should save");

    let tampered = crate::verify_published_shaping_generation(&config).expect("verify should run");
    assert_eq!(
        tampered.verified,
        Some(false),
        "a churned-world load->edit->save must FAIL (the recorded content digest)"
    );
    assert_eq!(
        tampered.failed_leg,
        Some(crate::ShapingCustodyLeg::FileMintMismatch),
        "§0b: the failing custody leg is NAMED (file↔mint)"
    );
}

/// Leg 2 POSITIVE (the churn — the deriver ``): a FRESH status whose on-disk
/// artifact lags the fresh mint by one churn (the world moved without a
/// republish) must be TOLERATED, so the tracker's effective-parent cache loads
/// (the UI-0 fix). A stale mismatch still fails (the edited-artifact test).
#[test]
fn verify_published_shaping_generation_tolerates_a_fresh_artifact_lag() {
    let config = flat_test_config("lqos-topology-verify-artifact-lag", 2);
    write_flat_test_devices(&config, &[("c1", 200.0)]);
    let canonical = crate::load_canonical_topology_state(&config);
    let health = TopologyAttachmentHealthStateFile::default();
    let overrides =
        TopologyOverridesFile::load_for_config(&config).expect("overrides should load");
    let artifacts =
        crate::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            &config,
            &canonical,
            &overrides,
            &health,
        )
        .expect("runtime build should succeed");
    crate::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("publish should succeed");

    // Move the WORLD (an override) WITHOUT republishing: the on-disk artifact
    // now lags the fresh mint by one churn (and the source moved too); the
    // status stays FRESH, so both lags are tolerated.
    fs::write(
        PathBuf::from(&config.lqos_directory).join("lqos_overrides.json"),
        serde_json::to_string_pretty(&json!({
            "persistent_devices": [],
            "circuit_adjustments": [{
                "type": "circuit_adjust_speed",
                "circuit_id": "c1",
                "min_download_bandwidth": 190.0,
                "max_download_bandwidth": 190.0,
                "min_upload_bandwidth": 95.0,
                "max_upload_bandwidth": 95.0
            }],
            "network_adjustments": []
        }))
        .expect("override json should serialize"),
    )
    .expect("overrides file should write");
    assert_eq!(
        crate::verify_published_shaping_generation(&config).expect("verify should run").verified,
        Some(true),
        "a fresh one-churn artifact lag must be tolerated (the UI-0 fix)"
    );
}

/// Leg 2 FRESH-TAMPER NEGATIVE (review ``): a tampered artifact on a
/// FRESH status with the SAME world (`generation == status.source_generation`)
/// must FAIL — the exact same-world check is NOT freshness-gated. (This is the
/// security teeth the blanket fresh-tolerate removed.)
#[test]
fn verify_published_shaping_generation_rejects_a_fresh_tampered_artifact() {
 // Card v2 i-a: readiness is the complete custody check
    // world ↔ generation ↔ artifact. An on-disk shaping_inputs.json that
    // diverges from the fresh mint must FAIL, even when the world is unchanged
    // (the relocated payload-tamper teeth).
    let config = flat_test_config("lqos-topology-verify-tamper", 2);
    write_flat_test_devices(&config, &[("c1", 200.0)]);
    let canonical = crate::load_canonical_topology_state(&config);
    let health = TopologyAttachmentHealthStateFile::default();
    let overrides =
        TopologyOverridesFile::load_for_config(&config).expect("overrides should load");
    let artifacts =
        crate::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            &config,
            &canonical,
            &overrides,
            &health,
        )
        .expect("runtime build should succeed");
    crate::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("publish should succeed");
    assert_eq!(
        crate::verify_published_shaping_generation(&config).expect("verify should run").verified,
        Some(true),
        "the untouched artifact verifies"
    );

    // Edit the on-disk ARTIFACT (not the world): the file leg must catch it.
    let mut on_disk =
        TopologyShapingInputsFile::load(&config).expect("on-disk shaping inputs should load");
    on_disk.circuits[0].effective_parent_node_name = "Tampered".to_string();
    on_disk
        .save(&config)
        .expect("tampered shaping inputs should save");
    // The WORLD is UNCHANGED (only the artifact is edited) and the status is
    // FRESH: the EXACT same-world check must catch the tamper (a fresh-tamper
 // negative — review.
    let tampered = crate::verify_published_shaping_generation(&config).expect("verify should run");
    assert_eq!(
        tampered.verified,
        Some(false),
        "an edited artifact diverged from the world must fail the custody check"
    );
    assert_eq!(
        tampered.failed_leg,
        Some(crate::ShapingCustodyLeg::FileMintMismatch),
        "§0b: the failing custody leg is NAMED (file↔mint)"
    );

    // Republishing the world restores the artifact and the agreement.
    crate::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("publish should succeed");
    assert_eq!(
        crate::verify_published_shaping_generation(&config).expect("verify should run").verified,
        Some(true),
        "republishing restores the artifact↔mint agreement"
    );
}

#[test]
fn verify_published_shaping_generation_rejects_edited_operator_view() {
    // Phase A §1 ruling 2: the operator-view match-assert is now a custody leg
    // INSIDE verify. A tampered operator-view file whose hash no longer equals
    // the published effective_generation must fail, even when the world and the
    // shaping inputs are untouched.
    let config = flat_test_config("lqos-topology-verify-operator-view", 2);
    write_flat_test_devices(&config, &[("c1", 200.0)]);
    let canonical = crate::load_canonical_topology_state(&config);
    let health = TopologyAttachmentHealthStateFile::default();
    let overrides =
        TopologyOverridesFile::load_for_config(&config).expect("overrides should load");
    let artifacts =
        crate::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            &config,
            &canonical,
            &overrides,
            &health,
        )
        .expect("runtime build should succeed");
    crate::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("publish should succeed");
    assert_eq!(
        crate::verify_published_shaping_generation(&config)
            .expect("verify should run")
            .verified,
        Some(true),
        "the untouched operator view verifies"
    );

    // Tamper the operator-view ARTIFACT (not the world): leg 3 must catch it.
    let operator_view_path = lqos_config::topology_operator_view_path(&config);
    let raw = std::fs::read_to_string(&operator_view_path).expect("operator view should read");
    let mut value: serde_json::Value =
        serde_json::from_str(&raw).expect("operator view should parse");
    value["tampered"] = serde_json::json!(true);
    std::fs::write(
        &operator_view_path,
        serde_json::to_string_pretty(&value).expect("operator view should serialize"),
    )
    .expect("operator view should write");
    let tampered = crate::verify_published_shaping_generation(&config).expect("verify should run");
    assert_eq!(
        tampered.verified,
        Some(false),
        "an edited operator view must fail the custody check"
    );
    assert_eq!(
        tampered.failed_leg,
        Some(crate::ShapingCustodyLeg::EffectiveMismatch),
        "§0b: the failing custody leg is NAMED (operator-view↔effective)"
    );

    // Republishing restores the operator view and the agreement.
    crate::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("publish should succeed");
    assert_eq!(
        crate::verify_published_shaping_generation(&config)
            .expect("verify should run")
            .verified,
        Some(true),
        "republishing restores the operator-view agreement"
    );
}

#[test]
fn operator_view_diff_classifies_key_vs_scalar() {
 // R2: the operator-view diag must distinguish a
    // top-level KEY difference from a scalar-only difference — a key-only diff
    // is SILENT in a value comparison and must not read as "no diff".
    use crate::operator_view_diff;
    let in_memory = serde_json::json!({
        "nodes": [1, 2],
        "rate_source": "base",
        "in_memory_only": true
    });
    let on_disk = serde_json::json!({
        "nodes": [1, 2],
        "rate_source": "override",
        "on_disk_only": true
    });
    let (key_only, scalar_diff) = operator_view_diff(&in_memory, &on_disk);
    assert_eq!(
        key_only,
        vec!["in_memory_only".to_string(), "on_disk_only".to_string()],
        "keys present on one side only are named"
    );
    assert_eq!(
        scalar_diff,
        vec!["rate_source".to_string()],
        "a shared key with a differing scalar is named"
    );
    // Identical values -> no difference at all.
    let (empty_keys, empty_scalars) = operator_view_diff(&in_memory, &in_memory);
    assert!(empty_keys.is_empty() && empty_scalars.is_empty());
}

#[test]
fn operator_view_nodes_diff_names_bounded_paths() {
 // R1/R2: the deeper `nodes` diff names PATHS (bounded,
    // first <= 8) + a count, and reports a deep `nodes_equal` bool.
    use crate::operator_view_nodes_diff;
    let in_memory = serde_json::json!({
        "nodes": [{"id": "c-1", "rate": 100.0, "name": "A"}, {"id": "c-2"}]
    });
    let on_disk = serde_json::json!({
        "nodes": [{"id": "c-1", "rate": 90.5, "name": "A"}, {"id": "c-2"}]
    });
    let (equal, paths, count) = operator_view_nodes_diff(&in_memory, &on_disk);
    assert!(!equal);
    assert_eq!(count, 1);
    assert_eq!(paths, vec!["nodes[*].rate".to_string()]);
    // Equal nodes -> empty + equal.
    let (equal, paths, count) = operator_view_nodes_diff(&in_memory, &in_memory);
    assert!(equal && paths.is_empty() && count == 0);
    // Bounded: 20 differing fields -> <= 8 paths but the FULL count.
    let mut a = serde_json::Map::new();
    let mut b = serde_json::Map::new();
    for k in 0..20 {
        a.insert(format!("f{k}"), serde_json::json!(1));
        b.insert(format!("f{k}"), serde_json::json!(2));
    }
    let many_a = serde_json::json!({ "nodes": [serde_json::Value::Object(a)] });
    let many_b = serde_json::json!({ "nodes": [serde_json::Value::Object(b)] });
    let (equal, paths, count) = operator_view_nodes_diff(&many_a, &many_b);
    assert!(!equal);
    assert_eq!(count, 20);
    assert!(paths.len() <= 8, "paths bounded to <= 8: {paths:?}");
 // a LENGTH-only difference must be named (zip hides it).
    let short = serde_json::json!({ "nodes": [{"id": "c-1"}] });
    let long = serde_json::json!({ "nodes": [{"id": "c-1"}, {"id": "c-2"}] });
    let (equal, paths, count) = operator_view_nodes_diff(&short, &long);
    assert!(!equal);
    assert!(count >= 1, "a length difference is counted");
    assert!(
        paths.iter().any(|path| path.starts_with("nodes (len")),
        "a length difference is named: {paths:?}"
    );
}

#[test]
fn published_effective_generation_matches_the_serialized_operator_view_file() {
 // (b) /: `effective_generation` must be the
    // CONTENT hash of the PERSISTED operator view (status == file) — the f32
    // non-identity made the in-memory `to_value` hash diverge from the file.
    let config = flat_test_config("lqos-topology-effective-gen-file", 2);
    write_flat_test_devices(&config, &[("c1", 200.0)]);
    let canonical = crate::load_canonical_topology_state(&config);
    let health = TopologyAttachmentHealthStateFile::default();
    let overrides =
        TopologyOverridesFile::load_for_config(&config).expect("overrides should load");
    let artifacts =
        crate::build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
            &config,
            &canonical,
            &overrides,
            &health,
        )
        .expect("runtime build should succeed");
    crate::publish_effective_topology_artifacts(&config, &artifacts)
        .expect("publish should succeed");

    let status = TopologyRuntimeStatusFile::load(&config).expect("status should load");
    let view_path = lqos_config::topology_operator_view_path(&config);
    let file_generation = lqos_config::compute_effective_network_file_generation(&view_path)
        .expect("the on-disk operator view must hash");
    assert_eq!(
        status.effective_generation, file_generation,
        "(b): the status effective_generation must equal the persisted file's generation"
    );
}
