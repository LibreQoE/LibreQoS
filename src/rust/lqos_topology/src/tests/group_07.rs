    fn dataplane_shaping_inputs() -> TopologyShapingInputsFile {
        serde_json::from_value(json!({
            "shaping_generation": "gen-1",
            "circuits": [
                {
                    "circuit_id": "c1", "circuit_name": "C1",
                    "effective_parent_node_name": "Tower 1",
                    "effective_parent_node_id": "tower-1",
                    "download_min_mbps": 10.0, "upload_min_mbps": 5.0,
                    "download_max_mbps": 100.0, "upload_max_mbps": 50.0,
                    "sqm_override": "cake diffserv4",
                    "devices": [
                        { "device_id": "d1", "device_name": "D1", "mac": "",
                          "ipv4": ["192.0.2.10/32"], "ipv6": [] }
                    ]
                }
            ]
        }))
        .expect("shaping inputs should deserialize")
    }

    fn dataplane_effective_network() -> Value {
        json!({
            "Tower 1": {
                "id": "tower-1",
                "name": "Tower 1",
                "downloadBandwidthMbps": 300.0,
                "downloadBandwidthMbpsMin": 30,
                "uploadBandwidthMbps": 50.0,
                "uploadBandwidthMbpsMin": 5,
                "children": {
                    "AP 1": {
                        "id": "ap-1",
                        "name": "AP 1",
                        "downloadBandwidthMbps": 100.0,
                        "downloadBandwidthMbpsMin": 10,
                        "uploadBandwidthMbps": 20.0,
                        "uploadBandwidthMbpsMin": 2,
                        "children": {}
                    }
                }
            }
        })
    }

    #[test]
    fn compile_transaction_emits_inner_upserts_with_synthetic_root() {
        // `lqos-74t.15.2` (A1): this fixture's c1 hangs under tower-1, and
        // ap-1 is CIRCUIT-LESS — but a childless node is LOAD-BEARING
        // (`analysis.rs:186` + `walk_place:347`) ⇒ ap-1 is KEPT (the F-LM
        // inert-chain elision is SUPERSEDED).
        let tx = compile_dataplane_transaction(
            &dataplane_effective_network(),
            &dataplane_shaping_inputs(),
        );

        assert_eq!(tx.generation, "gen-1");
        assert_eq!(tx.inner_ops.len(), 2, "the circuit-less ap-1 is KEPT (A1)");
        let inner_ids: Vec<&str> = tx
            .inner_ops
            .iter()
            .map(|op| match op {
                DataplaneInnerOp::Upsert { node_id, .. } => node_id.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(inner_ids, vec!["tower-1", "ap-1"], "tower-1 → ap-1 kept");
        assert_eq!(
            tx.inner_ops[0],
            DataplaneInnerOp::Upsert {
                node_id: "tower-1".into(),
                parent_node_id: DATA_PLANE_ROOT_NODE_ID.into(),
                download: DataplaneRatePair { commit_kbps: 30_000, ceil_kbps: 300_000 },
                upload: DataplaneRatePair { commit_kbps: 5_000, ceil_kbps: 50_000 },
            }
        );

        // The both-nodes pin (the pre-elision expectation) survives on
        // the circuit-bearing variant: c1 under ap-1 keeps the chain.
        let mut shaping = dataplane_shaping_inputs();
        shaping.circuits[0].effective_parent_node_id = "ap-1".into();
        let tx_full = compile_dataplane_transaction(&dataplane_effective_network(), &shaping);
        assert_eq!(tx_full.inner_ops.len(), 2);
        assert_eq!(
            tx_full.inner_ops[1],
            DataplaneInnerOp::Upsert {
                node_id: "ap-1".into(),
                parent_node_id: "tower-1".into(),
                download: DataplaneRatePair { commit_kbps: 10_000, ceil_kbps: 100_000 },
                upload: DataplaneRatePair { commit_kbps: 2_000, ceil_kbps: 20_000 },
            }
        );
    }

    #[test]
    fn compile_transaction_emits_leaf_upsert_and_mapping() {
        let tx = compile_dataplane_transaction(
            &dataplane_effective_network(),
            &dataplane_shaping_inputs(),
        );

        assert_eq!(tx.leaf_ops.len(), 1);
        assert_eq!(
            tx.leaf_ops[0],
            DataplaneLeafOp::Upsert {
                circuit_id: "c1".into(),
                parent_node_id: "tower-1".into(),
                download: DataplaneRatePair { commit_kbps: 10_000, ceil_kbps: 100_000 },
                upload: DataplaneRatePair { commit_kbps: 5_000, ceil_kbps: 50_000 },
                aqm: DataplaneLeafAqm::Cake { diffserv4: true, rtt_ms: None },
                admin_up: true,
            }
        );

        assert_eq!(tx.mapping_ops.len(), 1);
        assert_eq!(
            tx.mapping_ops[0],
            DataplaneMappingOp::Map {
                prefix_len: 32,
                ip: "192.0.2.10".parse().unwrap(),
                circuit_id: "c1".into(),
                device_id: "d1".into(),
            }
        );
    }

    /// T-arc §2.8: a circuit carrying a tunnel identity also emits a MapTunnel
    /// alongside its IP maps.
    #[test]
    fn compile_transaction_emits_map_tunnel_for_tunnel_identity() {
        let inputs: TopologyShapingInputsFile = serde_json::from_value(json!({
            "shaping_generation": "gen-tunnel",
            "circuits": [
                {
                    "circuit_id": "c1", "circuit_name": "C1",
                    "effective_parent_node_name": "Tower 1",
                    "effective_parent_node_id": "tower-1",
                    "download_min_mbps": 10.0, "upload_min_mbps": 5.0,
                    "download_max_mbps": 100.0, "upload_max_mbps": 50.0,
                    "tunnel_identity": { "vni": 100, "vlan": 64 },
                    "devices": [
                        { "device_id": "d1", "device_name": "D1", "mac": "",
                          "ipv4": ["192.0.2.10"], "ipv6": [] }
                    ]
                }
            ]
        }))
        .expect("shaping inputs should deserialize");
        let tx = compile_dataplane_transaction(&dataplane_effective_network(), &inputs);
        assert!(
            tx.mapping_ops.iter().any(|op| matches!(
                op,
                DataplaneMappingOp::MapTunnel { vni: 100, vlan: 64, circuit } if circuit == "c1"
            )),
            "the tunnel identity must emit a MapTunnel: {:?}",
            tx.mapping_ops
        );
    }

    /// Phase C S2: a circuit carrying a vlan identity emits a MapVlan, and an
    /// mpls identity a MapMpls (alongside any IP maps).
    #[test]
    fn compile_transaction_emits_map_vlan_and_mpls() {
        let inputs: TopologyShapingInputsFile = serde_json::from_value(json!({
            "shaping_generation": "gen-vlan-mpls",
            "circuits": [
                {
                    "circuit_id": "c1", "circuit_name": "C1",
                    "effective_parent_node_name": "Tower 1",
                    "effective_parent_node_id": "tower-1",
                    "download_min_mbps": 10.0, "upload_min_mbps": 5.0,
                    "download_max_mbps": 100.0, "upload_max_mbps": 50.0,
                    "vlan_identity": 64,
                    "devices": []
                },
                {
                    "circuit_id": "c2", "circuit_name": "C2",
                    "effective_parent_node_name": "Tower 1",
                    "effective_parent_node_id": "tower-1",
                    "download_min_mbps": 10.0, "upload_min_mbps": 5.0,
                    "download_max_mbps": 100.0, "upload_max_mbps": 50.0,
                    "mpls_identity": 1048575,
                    "devices": []
                }
            ]
        }))
        .expect("shaping inputs should deserialize");
        let tx = compile_dataplane_transaction(&dataplane_effective_network(), &inputs);
        assert!(
            tx.mapping_ops.iter().any(|op| matches!(
                op,
                DataplaneMappingOp::MapVlan { vlan: 64, circuit } if circuit == "c1"
            )),
            "the vlan identity must emit a MapVlan: {:?}",
            tx.mapping_ops
        );
        assert!(
            tx.mapping_ops.iter().any(|op| matches!(
                op,
                DataplaneMappingOp::MapMpls { label: 1_048_575, circuit } if circuit == "c2"
            )),
            "the mpls identity must emit a MapMpls: {:?}",
            tx.mapping_ops
        );
    }

    #[test]
    fn aqm_override_mapping_is_simplified() {
        assert_eq!(aqm_from_override(None), DataplaneLeafAqm::None);
        assert_eq!(aqm_from_override(Some("")), DataplaneLeafAqm::None);
        assert_eq!(aqm_from_override(Some("none")), DataplaneLeafAqm::None);
        assert_eq!(
            aqm_from_override(Some("fq_codel")),
            DataplaneLeafAqm::None,
            "the bridge AQM grammar is none|cake only"
        );
        assert_eq!(
            aqm_from_override(Some("cake")),
            DataplaneLeafAqm::Cake { diffserv4: false, rtt_ms: None }
        );
        assert_eq!(
            aqm_from_override(Some("cake rtt300")),
            DataplaneLeafAqm::Cake { diffserv4: false, rtt_ms: Some(300) }
        );
    }

/// R0 (remove_lpm_completeness_card, fresh-#12: a BARE device
/// IP is a host address — /32 v4, /128 v6 — the same normalization the
/// runtime_inputs normalizers apply. Before R0 the bare form was
/// dropped SILENTLY (split_once('/')? returned None) and the whole
/// legacy pipeline compiled zero maps. Garbage still skips, but now
/// with a named warn.
#[test]
fn r0_bare_ips_compile_as_host_prefixes_and_garbage_warns() {
    let inputs: TopologyShapingInputsFile = serde_json::from_value(json!({
        "shaping_generation": "gen-r0",
        "circuits": [
            {
                "circuit_id": "c1", "circuit_name": "C1",
                "effective_parent_node_name": "Tower 1",
                "effective_parent_node_id": "tower-1",
                "download_min_mbps": 10.0, "upload_min_mbps": 5.0,
                "download_max_mbps": 100.0, "upload_max_mbps": 50.0,
                "devices": [
                    { "device_id": "d1", "device_name": "D1", "mac": "",
                      "ipv4": ["192.0.2.10", "198.51.100.7/28"],
                      "ipv6": ["2001:db8::1"] }
                ]
            }
        ]
    }))
    .expect("shaping inputs should deserialize");
    let tx = compile_dataplane_transaction(&dataplane_effective_network(), &inputs);

    let mut maps: Vec<(u8, String, String)> = tx
        .mapping_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneMappingOp::Map { prefix_len, ip, circuit_id, .. } => {
                Some((*prefix_len, ip.to_string(), circuit_id.clone()))
            }
            _ => None,
        })
        .collect();
    maps.sort();
    assert_eq!(
        maps,
        vec![
            (28u8, "198.51.100.7".to_string(), "c1".to_string()),
            (32u8, "192.0.2.10".to_string(), "c1".to_string()),
            (128u8, "2001:db8::1".to_string(), "c1".to_string()),
        ],
        "bare v4 -> /32, bare v6 -> /128, explicit CIDR kept"
    );
}

/// F-LM fixtures (lkg_boot_mirror_card §9): the rig world VERBATIM —
/// the s10-a chain (fc12ff82→root, c99483a3→fc12ff82) + the drill-era
/// s10-b chain (3a05f3f9→root, e4c5b25b→3a05f3f9, NO circuits), with
/// c1 under c99483a3 (the restored-CSV desired). These mirror the
/// rob-dpdk classify-unit fixtures so both compilers pin the SAME
/// fixture set (the §9 drift bound).
fn f_lm_effective_network() -> Value {
    json!({
        "s10-a": {
            "id": "libreqos:legacy-network-json:node:fc12ff8270a70366",
            "name": "s10-a",
            "downloadBandwidthMbps": 10000.0,
            "uploadBandwidthMbps": 10000.0,
            "children": {
                "s10-a-ap0": {
                    "id": "libreqos:legacy-network-json:node:c99483a36594a604",
                    "name": "s10-a-ap0",
                    "downloadBandwidthMbps": 10000.0,
                    "uploadBandwidthMbps": 10000.0,
                    "children": {}
                }
            }
        },
        "s10-b": {
            "id": "libreqos:legacy-network-json:node:3a05f3f9c95a92be",
            "name": "s10-b",
            "downloadBandwidthMbps": 10000.0,
            "uploadBandwidthMbps": 10000.0,
            "children": {
                "s10-b-ap0": {
                    "id": "libreqos:legacy-network-json:node:e4c5b25cbd2f1a64",
                    "name": "s10-b-ap0",
                    "downloadBandwidthMbps": 10000.0,
                    "uploadBandwidthMbps": 10000.0,
                    "children": {}
                }
            }
        }
    })
}

fn f_lm_shaping(circuits: Value) -> TopologyShapingInputsFile {
    serde_json::from_value(json!({
        "shaping_generation": "gen-f-lm",
        "circuits": circuits
    }))
    .expect("f_lm shaping inputs deserialize")
}

fn f_lm_c1_only() -> Value {
    json!([
        {
            "circuit_id": "c1", "circuit_name": "C1",
            "effective_parent_node_name": "s10-a-ap0",
            "effective_parent_node_id": "libreqos:legacy-network-json:node:c99483a36594a604",
            "download_min_mbps": 100.0, "upload_min_mbps": 50.0,
            "download_max_mbps": 200.0, "upload_max_mbps": 100.0,
            "devices": [
                { "device_id": "d1", "device_name": "D1", "mac": "",
                  "ipv4": ["100.97.0.1/32"], "ipv6": [] }
            ]
        }
    ])
}

/// `lqos-74t.15.2` (A1) — Gate 1 SUPERSEDED: the compiled desired carries
/// BOTH chains — the empty s10-b chain's childless leaf is LOAD-BEARING
/// (`analysis.rs:186` + `walk_place:347`), so it is KEPT, not elided. The
/// F-LM inert-chain ELISION WAS the `AnalysisEmissionDivergence` class.
#[test]
fn f_lm_empty_site_chain_kept_from_desired() {
    let tx = compile_dataplane_transaction(&f_lm_effective_network(), &f_lm_shaping(f_lm_c1_only()));
    let inner_ids: Vec<&str> = tx
        .inner_ops
        .iter()
        .map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, .. } => node_id.as_str(),
            _ => "?",
        })
        .collect();
    assert_eq!(
        inner_ids,
        vec![
            "libreqos:legacy-network-json:node:fc12ff8270a70366",
            "libreqos:legacy-network-json:node:c99483a36594a604",
            "libreqos:legacy-network-json:node:3a05f3f9c95a92be",
            "libreqos:legacy-network-json:node:e4c5b25cbd2f1a64",
        ],
        "(A1): BOTH chains carried — the s10-b chain's childless leaf is load-bearing"
    );
    assert_eq!(
        tx.inner_ops
            .iter()
            .map(|op| match op {
                DataplaneInnerOp::Upsert { parent_node_id, .. } => parent_node_id.as_str(),
                _ => "?",
            })
            .collect::<Vec<_>>(),
        vec![
            DATA_PLANE_ROOT_NODE_ID,
            "libreqos:legacy-network-json:node:fc12ff8270a70366",
            DATA_PLANE_ROOT_NODE_ID,
            "libreqos:legacy-network-json:node:3a05f3f9c95a92be",
        ],
        "root attachment + chain wiring preserved for BOTH chains"
    );
    assert_eq!(tx.leaf_ops.len(), 1, "c1 unchanged");
}

/// `lqos-74t.15.2` (A1) — Gate 2 SUPERSEDED: an INTERMEDIATE on a circuit's
/// ancestry KEEPS (eliding it would dangle the leaf parent) **AND** the
/// circuit-less sibling branch (`empty-branch`/`deeper-empty`) ALSO KEEPS —
/// its childless leaf is load-bearing (`walk_place:347`).
#[test]
fn f_lm_intermediate_and_empty_branch_is_kept() {
    let network = json!({
        "root-site": {
            "id": "root-site", "name": "root-site",
            "downloadBandwidthMbps": 1000.0, "uploadBandwidthMbps": 1000.0,
            "children": {
                "mid": {
                    "id": "mid", "name": "mid",
                    "downloadBandwidthMbps": 500.0, "uploadBandwidthMbps": 500.0,
                    "children": {
                        "leaf-ap": {
                            "id": "leaf-ap", "name": "leaf-ap",
                            "downloadBandwidthMbps": 100.0, "uploadBandwidthMbps": 100.0,
                            "children": {}
                        }
                    }
                },
                "empty-branch": {
                    "id": "empty-branch", "name": "empty-branch",
                    "downloadBandwidthMbps": 100.0, "uploadBandwidthMbps": 100.0,
                    "children": {
                        "deeper-empty": {
                            "id": "deeper-empty", "name": "deeper-empty",
                            "downloadBandwidthMbps": 50.0, "uploadBandwidthMbps": 50.0,
                            "children": {}
                        }
                    }
                }
            }
        }
    });
    let shaping = f_lm_shaping(json!([
        {
            "circuit_id": "c1", "circuit_name": "C1",
            "effective_parent_node_name": "leaf-ap",
            "effective_parent_node_id": "leaf-ap",
            "download_min_mbps": 10.0, "upload_min_mbps": 5.0,
            "download_max_mbps": 100.0, "upload_max_mbps": 50.0,
            "devices": []
        }
    ]));
    let tx = compile_dataplane_transaction(&network, &shaping);
    let inner_ids: Vec<&str> = tx
        .inner_ops
        .iter()
        .map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, .. } => node_id.as_str(),
            _ => "?",
        })
        .collect();
    assert_eq!(
        inner_ids,
        vec!["root-site", "empty-branch", "deeper-empty", "mid", "leaf-ap"],
        "(A1): the circuit-less sibling branch is KEPT too (its childless leaf is load-bearing)"
    );
}

/// Gate 3 — §7 symmetry: a circuit ADD under a previously-elided site
/// re-materializes the FULL chain in the same compile (the N1
/// add-carrying shape).
#[test]
fn f_lm_circuit_add_rematerializes_full_chain() {
    let shaping = f_lm_shaping(json!([
        {
            "circuit_id": "c1", "circuit_name": "C1",
            "effective_parent_node_name": "s10-a-ap0",
            "effective_parent_node_id": "libreqos:legacy-network-json:node:c99483a36594a604",
            "download_min_mbps": 100.0, "upload_min_mbps": 50.0,
            "download_max_mbps": 200.0, "upload_max_mbps": 100.0,
            "devices": []
        },
        {
            "circuit_id": "c5", "circuit_name": "C5",
            "effective_parent_node_name": "s10-b-ap0",
            "effective_parent_node_id": "libreqos:legacy-network-json:node:e4c5b25cbd2f1a64",
            "download_min_mbps": 10.0, "upload_min_mbps": 5.0,
            "download_max_mbps": 20.0, "upload_max_mbps": 10.0,
            "devices": []
        }
    ]));
    let tx = compile_dataplane_transaction(&f_lm_effective_network(), &shaping);
    let inner_ids: Vec<&str> = tx
        .inner_ops
        .iter()
        .map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, .. } => node_id.as_str(),
            _ => "?",
        })
        .collect();
    assert_eq!(
        inner_ids,
        vec![
            "libreqos:legacy-network-json:node:fc12ff8270a70366",
            "libreqos:legacy-network-json:node:c99483a36594a604",
            "libreqos:legacy-network-json:node:3a05f3f9c95a92be",
            "libreqos:legacy-network-json:node:e4c5b25cbd2f1a64",
        ],
        "the previously-elided chain re-materializes complete"
    );
    assert_eq!(tx.leaf_ops.len(), 2, "c1 + c5");
}

/// `lqos-74t.15.2` (A1) — Gate 4 SUPERSEDED: adding a circuit-less tower now
/// ADDS the tower's shell (its childless node is load-bearing) ⇒ the WHOLE tx is
/// NOT byte-identical — BUT every CIRCUIT-BEARING op is PRESERVED (the no-op
/// that mattered).
#[test]
fn f_lm_circuit_bearing_part_is_preserved() {
    let mut network_with_empty = dataplane_effective_network();
    network_with_empty["Empty Tower"] = json!({
        "id": "empty-tower", "name": "Empty Tower",
        "downloadBandwidthMbps": 100.0, "uploadBandwidthMbps": 100.0,
        "children": {}
    });
    let with_empty = compile_dataplane_transaction(&network_with_empty, &dataplane_shaping_inputs());
    let bare = compile_dataplane_transaction(&dataplane_effective_network(), &dataplane_shaping_inputs());
    let bare_ids: Vec<String> = bare
        .inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, .. } => Some(node_id.clone()),
            _ => None,
        })
        .collect();
    let with_ids: Vec<String> = with_empty
        .inner_ops
        .iter()
        .filter_map(|op| match op {
            DataplaneInnerOp::Upsert { node_id, .. } => Some(node_id.clone()),
            _ => None,
        })
        .collect();
    for id in &bare_ids {
        assert!(
            with_ids.contains(id),
            "the circuit-bearing op {id} SURVIVES the added empty tower"
        );
    }
    assert!(
        with_ids.contains(&"empty-tower".to_string()),
        "(A1): the childless empty tower IS emitted: {with_ids:?}"
    );
}

    /// `lqos-74t.15.2` (A1) — CROSS-EMITTER parity (production side). The SAME
    /// shared fixture (`docs/test_fixtures/a1_cross_emitter/{world,shaping}.json`)
    /// the guard crate pins: `Empty Site` is a NON-`orphans` childless,
    /// circuit-less node and MUST be kept. Pins the production golden
    /// (`production_tx.json`); the guard crate asserts its own emission is
    /// byte-identical to this golden (a golden PER CRATE, one shared fixture —
    /// the two crates share no dependency, control_channel_card §2/§3.9).
    #[test]
    fn a1_cross_emitter_production_emits_the_shared_fixture() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../docs/test_fixtures/a1_cross_emitter");
        let world: Value = serde_json::from_str(
            &fs::read_to_string(dir.join("world.json")).expect("world.json readable"),
        )
        .expect("world parses");
        let shaping: TopologyShapingInputsFile = serde_json::from_value(
            serde_json::from_str(
                &fs::read_to_string(dir.join("shaping.json")).expect("shaping.json readable"),
            )
            .expect("shaping parses"),
        )
        .expect("shaping deserializes");
        let tx = compile_dataplane_transaction(&world, &shaping);
        let ids: Vec<&str> = tx
            .inner_ops
            .iter()
            .map(|op| match op {
                DataplaneInnerOp::Upsert { node_id, .. } => node_id.as_str(),
                _ => "?",
            })
            .collect();
        assert!(
            ids.contains(&"empty-site"),
            "(A1) the NON-orphans childless, circuit-less node is KEPT: {ids:?}"
        );
        assert!(
            ids.contains(&"throttled-site"),
            "(A1) the Min!=bandwidth childless node is KEPT too: {ids:?}"
        );
        assert!(ids.contains(&"bearing-site"), "the circuit-bearing sibling is kept");
        let canon = serde_json::to_string(&tx).expect("production tx serializes");
        eprintln!("PRODUCTION_TX={canon}");
        let golden = dir.join("production_tx.json");
        if let Ok(pinned) = fs::read_to_string(&golden) {
            assert_eq!(
                canon,
                pinned.trim_end(),
                "production emission drifted from its golden"
            );
        }
    }
