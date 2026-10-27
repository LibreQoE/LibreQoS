// K1 §1.1/§1.2: the RADIUS dynamic-circuit store folds into shaping_inputs at
// the DEVICE-ROW level, so the compiler emits per-IP Map ops and the broker's
// diff can add/remove the circuit on the bridge.

use std::net::Ipv4Addr;

fn dynamic_circuit(
    circuit_id: &str,
    parent: &str,
    ip: &str,
    prefix: u32,
) -> lqos_config::DynamicCircuit {
    lqos_config::DynamicCircuit {
        shaped: lqos_config::ShapedDevice {
            circuit_id: circuit_id.to_string(),
            circuit_name: circuit_id.to_string(),
            device_id: format!("{circuit_id}-dev"),
            device_name: circuit_id.to_string(),
            parent_node: parent.to_string(),
            parent_node_id: None,
            ipv4: vec![(ip.parse::<Ipv4Addr>().expect("ipv4"), prefix)],
            download_min_mbps: 10.0,
            upload_min_mbps: 5.0,
            download_max_mbps: 100.0,
            upload_max_mbps: 50.0,
            ..Default::default()
        },
        last_seen_unix: 1,
    }
}

fn write_dynamic_store(config: &Config, circuits: Vec<lqos_config::DynamicCircuit>) {
    let store = lqos_config::DynamicCircuitsFile {
        schema_version: 1,
        circuits,
    };
    std::fs::write(
        lqos_config::dynamic_circuits_path(config),
        serde_json::to_string(&store).expect("store serializes"),
    )
    .expect("dynamic store should write");
}

#[test]
fn k1_dynamic_fold_emits_device_rows_and_map_ops() {
    let config = tunnel_test_config("k1-dynamic-fold");
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    write_dynamic_store(
        &config,
        vec![dynamic_circuit("dyn-1", "site-1", "10.0.0.5", 32)],
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let circuit = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dyn-1")
        .expect("the dynamic circuit must fold into shaping inputs");
    assert_eq!(circuit.devices.len(), 1, "device row must fold");
    assert_eq!(circuit.devices[0].ipv4, vec!["10.0.0.5".to_string()]);
    assert_eq!(
        circuit.effective_parent_node_id,
        "tunnel-circuits:site:site-1",
        "the parent name must resolve through the effective tree"
    );

    // The leaf alone is blind — the Map arm proves classification.
    let network = artifacts
        .effective_network
        .clone()
        .expect("effective network should exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    assert!(
        tx.leaf_ops.iter().any(|op| matches!(
            op,
            DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "dyn-1"
        )),
        "the dynamic circuit must emit a leaf: {:?}",
        tx.leaf_ops
    );
    assert!(
        tx.mapping_ops.iter().any(|op| matches!(
            op,
            DataplaneMappingOp::Map { circuit_id, ip, .. }
                if circuit_id == "dyn-1" && ip.to_string() == "10.0.0.5"
        )),
        "the dynamic circuit must emit a Map op for its device IP: {:?}",
        tx.mapping_ops
    );
}

/// Dynamic-ranges M5 (conservation + root/site budget): the unconfirmed range
/// leaf plus N promoted /32 circuits all sit under ONE parent. The compiler
/// must emit exactly N+1 leaves and N+1 Maps, each promoted IP resolving to
/// exactly one /32 circuit (the /32 strictly shadows the range /24), with the
/// root→site ancestry materialized exactly once. The promoted commits may
/// transiently over-subscribe the site (disclosed), so this gate does NOT
/// assert a sum bound — it asserts conservation of identity/charge paths.
#[test]
fn dynamic_range_conservation_and_root_site_budget() {
    use lqos_config::{
        DynamicCircuitRangeRule, TopologyShapingCircuitInput, TopologyShapingDeviceInput,
        TopologyShapingInputsFile,
    };
    let network = json!({
        "site-a": {
            "id": "site-a-id",
            "name": "site-a",
            "downloadBandwidthMbps": 1000,
            "uploadBandwidthMbps": 1000,
            "children": {}
        }
    });

    let promoted_ips = ["10.60.0.5", "10.60.0.9", "10.60.0.42"];
    let mut inputs = TopologyShapingInputsFile {
        schema_version: 1,
        ..Default::default()
    };
    for (index, ip) in promoted_ips.iter().enumerate() {
        inputs.circuits.push(TopologyShapingCircuitInput {
            circuit_id: format!("dynamic-range:dr-pool:{ip}"),
            circuit_name: format!("dynamic-range:dr-pool:{ip}"),
            effective_parent_node_name: "site-a".to_string(),
            effective_parent_node_id: "site-a-id".to_string(),
            download_min_mbps: 1.0,
            upload_min_mbps: 1.0,
            download_max_mbps: 10.0,
            upload_max_mbps: 10.0,
            devices: vec![TopologyShapingDeviceInput {
                device_id: format!("dynamic-range:dr-pool:{ip}"),
                device_name: format!("dynamic-range:dr-pool:{ip}"),
                ipv4: vec![format!("{ip}/32")],
                ..TopologyShapingDeviceInput::default()
            }],
            ..TopologyShapingCircuitInput::default()
        });
        let _ = index;
    }

    let range = DynamicCircuitRangeRule {
        name: "dr-pool".to_string(),
        ip_range: "10.60.0.0/24".parse().expect("cidr"),
        download_min_mbps: 1.0,
        upload_min_mbps: 1.0,
        download_max_mbps: 10.0,
        upload_max_mbps: 10.0,
        attach_to: "site-a".to_string(),
    };

    let tx = crate::compile_dataplane_transaction_with_ranges(&network, &inputs, &[range]);

    // N promoted leaves + 1 range leaf.
    let range_leaf = tx
        .leaf_ops
        .iter()
        .filter(|op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "dynamic-range:dr-pool"))
        .count();
    let promoted_leaves = tx
        .leaf_ops
        .iter()
        .filter(|op| matches!(op, DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id.starts_with("dynamic-range:dr-pool:")))
        .count();
    assert_eq!(range_leaf, 1, "exactly one unconfirmed range leaf");
    assert_eq!(
        promoted_leaves,
        promoted_ips.len(),
        "one promoted leaf per circuit"
    );
    assert_eq!(
        tx.leaf_ops.len(),
        promoted_ips.len() + 1,
        "no other leaves in this world"
    );

    // Exactly one /32 Map per promoted IP, plus one /24 range Map.
    for ip in promoted_ips {
        let count = tx
            .mapping_ops
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    DataplaneMappingOp::Map { prefix_len: 32, ip: mapped, .. }
                        if mapped.to_string() == ip
                )
            })
            .count();
        assert_eq!(count, 1, "{ip} must map to exactly one /32 circuit");
    }
    let range_map = tx
        .mapping_ops
        .iter()
        .filter(|op| {
            matches!(
                op,
                DataplaneMappingOp::Map { prefix_len: 24, ip, .. } if ip.to_string() == "10.60.0.0"
            )
        })
        .count();
    assert_eq!(range_map, 1, "the range /24 Map must be present");
    assert_eq!(
        tx.mapping_ops.len(),
        promoted_ips.len() + 1,
        "conservation: N promoted /32 Maps + one range /24 Map"
    );

    // Root→site ancestry materialized exactly once (the budget chain): the
    // single top-level site hangs directly off the dataplane root.
    let site_ops = tx
        .inner_ops
        .iter()
        .filter(|op| {
            matches!(
                op,
                DataplaneInnerOp::Upsert { node_id, parent_node_id, .. }
                    if node_id == "site-a-id" && parent_node_id == DATA_PLANE_ROOT_NODE_ID
            )
        })
        .count();
    assert_eq!(site_ops, 1, "the site inner op appears exactly once under root");
    assert_eq!(
        tx.inner_ops.len(),
        1,
        "exactly one inner op (the shared site budget chain)"
    );
}

#[test]
fn k1_dynamic_fold_rejects_base_collision() {
    let config = tunnel_test_config("k1-dynamic-collision");
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    // A provisioned base row with the same circuit id.
    let mut csv = String::from(
        "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
    );
    csv.push_str("\"dyn-1\",\"dyn-1\",\"base-dev\",\"Base\",\"site-1\",\"tunnel-circuits:site:site-1\",\"\",\"\",\"10.0.0.9/32\",\"\",\"10\",\"5\",\"100\",\"50\",\"\"\n");
    std::fs::write(
        ConfigShapedDevices::path_for_config(&config),
        csv,
    )
    .expect("base csv should write");
    write_dynamic_store(
        &config,
        vec![dynamic_circuit("dyn-1", "site-1", "10.0.0.5", 32)],
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let circuit = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dyn-1")
        .expect("the provisioned circuit survives");
    assert_eq!(
        circuit.devices.len(),
        1,
        "the provisioned row is kept; the RADIUS shadow is rejected"
    );
    assert_eq!(
        circuit.devices[0].ipv4,
        vec!["10.0.0.9".to_string()],
        "the base device IP must win, not the RADIUS shadow"
    );
}

#[test]
fn k1_dynamic_fold_defers_unresolvable_parent() {
    let config = tunnel_test_config("k1-dynamic-deferred");
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    write_dynamic_store(
        &config,
        vec![dynamic_circuit("dyn-orphan", "no-such-site", "10.0.0.7", 32)],
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    assert!(
        !inputs
            .circuits
            .iter()
            .any(|circuit| circuit.circuit_id == "dyn-orphan"),
        "an unresolvable-parent dynamic circuit must NOT fold (loud-deferred)"
    );
    assert!(
        inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("shaping_deferred")),
        "the deferral must be named loudly: {:?}",
        inputs.warnings
    );
}

/// K1 §7: a fallback-supplied parent/anchor id set to the
/// literal node NAME (the drill's misconfiguration) must defer loudly, never
/// emit an empty effective parent the bridge would reject. Name-only fallback
/// (the working shape) is covered by `k1_dynamic_fold_emits_device_rows_and_map_ops`.
#[test]
fn k1_dynamic_fold_defers_unresolvable_fallback_identity() {
    let config = tunnel_test_config("k1-dynamic-fallback-deferred");
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let mut circuit = dynamic_circuit("dyn-fallback", "site-1", "10.0.0.8", 32);
    // The fallback ids carry the node NAME, not its effective id.
    circuit.shaped.parent_node_id = Some("site-1".to_string());
    circuit.shaped.anchor_node_id = Some("site-1".to_string());
    write_dynamic_store(&config, vec![circuit]);

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    assert!(
        !inputs
            .circuits
            .iter()
            .any(|circuit| circuit.circuit_id == "dyn-fallback"),
        "an unresolvable fallback id must NOT fold (loud-deferred)"
    );
    assert!(
        inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("shaping_deferred")),
        "the deferral must be named loudly: {:?}",
        inputs.warnings
    );

    // Zero transaction ops for the deferred row (the leaf AND its Map).
    let network = artifacts
        .effective_network
        .clone()
        .expect("effective network should exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    assert!(
        !tx.leaf_ops.iter().any(|op| matches!(
            op,
            DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "dyn-fallback"
        )),
        "a deferred row must emit no leaf: {:?}",
        tx.leaf_ops
    );
    assert!(
        !tx.mapping_ops.iter().any(|op| matches!(
            op,
            DataplaneMappingOp::Map { circuit_id, .. } if circuit_id == "dyn-fallback"
        )),
        "a deferred row must emit no Map: {:?}",
        tx.mapping_ops
    );
}
