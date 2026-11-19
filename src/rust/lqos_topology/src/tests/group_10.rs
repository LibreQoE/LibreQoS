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

/// The root node NAME of the exported effective tree: the top-level object's
/// `name` (the same precedence `collect_exported_effective_nodes` uses), else its
/// key. This is what a blank rule attachment means for a promoted row (u12).
fn exported_root_node_name(artifacts: &EffectiveTopologyArtifacts) -> String {
    let export = artifacts
        .effective_network
        .as_ref()
        .expect("the proof world exports an effective network");
    let (key, node) = export
        .as_object()
        .and_then(|map| map.iter().next())
        .expect("the export is one root object");
    node.get("name")
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| key.clone())
}

/// The hermetic world the promoted-row gates fold against: one `/24` rule
/// attached to `site-1`, nothing ignored, so a store row is the only variable.
fn promoted_row_world(prefix: &str) -> (Config, EffectiveTopologyArtifacts) {
    let mut config = range_test_config(
        prefix,
        vec![range_rule("range-24", "192.168.66.0/24", "site-1")],
    );
    config.ip_ranges.ignore_subnets = Vec::new();
    let artifacts = tunnel_artifacts(&config, &range_canonical("site-1", 1_000, 1_000));
    (config, artifacts)
}

/// u12 A4: a PROMOTED store row — the actor's `dynamic-range:<rule>:<ip>`
/// identity carrying a host `/32` — folded through the real
/// `build_shaping_inputs`, must produce an ordinary circuit: a leaf plus a host
/// `Map`, as a SIBLING of the range fallback.
///
/// This is the assertion the live u11 run was missing: there the row was written
/// with `parent_node = "dynamic-range:range-24"`, and because a circuit id is not
/// a node the fold deferred it on every publish, so the promotion existed in the
/// store and nowhere else.
#[test]
fn a_promoted_host_row_folds_into_a_leaf_and_a_host_map() {
    let (config, artifacts) = promoted_row_world("u12-promoted-host");
    // Exactly the row the actor now writes for this rule: the promoted id, the
    // rule's RESOLVED node name, one /32 mapping, the rule's rates.
    write_dynamic_store(
        &config,
        vec![dynamic_circuit(
            "dynamic-range:range-24:192.168.66.4",
            "site-1",
            "192.168.66.4",
            32,
        )],
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let promoted = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:range-24:192.168.66.4")
        .expect("the promoted row must fold: a circuit-id parent is what deferred it before u12");
    assert_eq!(promoted.devices.len(), 1, "one promoted host, one device row");
    assert_eq!(promoted.devices[0].ipv4, vec!["192.168.66.4".to_string()]);
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic-range:range-24:192.168.66.4")),
        "the promoted row must not be deferred: {:?}",
        inputs.warnings
    );
    // u13: the row carries its parent by NAME. The only id in the folded output
    // is the one the fold proved against the export itself; the store row
    // supplied none, because the id the raw `network.json` view reports is not
    // provable there (see `a_promoted_host_row_carrying_an_unprovable_parent_id_defers`).
    assert_eq!(
        promoted.logical_parent_node_name.as_deref(),
        Some("site-1"),
        "the parent rides by name"
    );
    assert_eq!(
        promoted.logical_parent_node_id.as_deref(),
        Some(promoted.effective_parent_node_id.as_str()),
        "the folded row's parent id is the fold's own resolution, not a carried-in id"
    );
    assert!(
        !promoted.effective_parent_node_id.contains("dynamic-range:"),
        "a circuit id is never a parent node: {}",
        promoted.effective_parent_node_id
    );
    // Sibling, not child: the range fallback for the same rule resolves the same
    // parent, which is what "a promoted host lands beside the fallback" means.
    let fallback = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:range-24")
        .expect("the range fallback still folds");
    assert_eq!(
        promoted.effective_parent_node_id, fallback.effective_parent_node_id,
        "the promoted circuit is a SIBLING of its range fallback"
    );

    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    assert!(
        tx.leaf_ops.iter().any(|op| matches!(
            op,
            DataplaneLeafOp::Upsert { circuit_id, .. }
                if circuit_id == "dynamic-range:range-24:192.168.66.4"
        )),
        "the promoted circuit must emit an ordinary leaf: {:?}",
        tx.leaf_ops
    );
    assert!(
        tx.mapping_ops.iter().any(|op| matches!(
            op,
            DataplaneMappingOp::Map { circuit_id, ip, prefix_len: 32, .. }
                if circuit_id == "dynamic-range:range-24:192.168.66.4"
                    && ip.to_string() == "192.168.66.4"
        )),
        "the promoted circuit must emit its /32 Map: {:?}",
        tx.mapping_ops
    );
}

/// u12 A4, the name-carrying half: a store row that names the export's ROOT NODE
/// still folds by name (u15 keeps that path — named-attach semantics are
/// unchanged by the root-sentinel ruling). The actor no longer writes this shape:
/// it writes the fold's `Root` sentinel
/// (`a_promoted_host_row_written_under_the_root_sentinel_folds_at_the_wire_root`),
/// but a row an operator or an older build wrote this way must keep working.
#[test]
fn a_promoted_host_row_naming_the_exported_root_node_folds_under_it() {
    let mut config = range_test_config(
        "u12-promoted-root",
        vec![range_rule("range-24", "192.168.66.0/24", "")],
    );
    config.ip_ranges.ignore_subnets = Vec::new();
    let canonical = range_canonical("site-1", 1_000, 1_000);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let root_name = exported_root_node_name(&artifacts);
    write_dynamic_store(
        &config,
        vec![dynamic_circuit(
            "dynamic-range:range-24:192.168.66.5",
            &root_name,
            "192.168.66.5",
            32,
        )],
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let promoted = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:range-24:192.168.66.5")
        .expect("the row written with the root NODE name must fold");
    // PINNED (REV-T3): the fixture's one site is `tunnel-circuits:site:site-1`, a
    // literal — not a value re-derived by mirroring the fold's own resolution.
    assert_eq!(
        promoted.effective_parent_node_id, "tunnel-circuits:site:site-1",
        "a row naming the exported root NODE resolves by name to that node"
    );
    assert_eq!(promoted.effective_parent_node_name, "site-1");
    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    assert!(
        tx.leaf_ops.iter().any(|op| matches!(
            op,
            DataplaneLeafOp::Upsert { circuit_id, .. }
                if circuit_id == "dynamic-range:range-24:192.168.66.5"
        )),
        "the root-attached promoted circuit must emit a leaf: {:?}",
        tx.leaf_ops
    );
}

/// u15 A3 + driver ruling (2026-11-17), the gate the two live runs were missing:
/// the row the ACTOR writes for a blank `attach_to` — the fold's own `Root`
/// sentinel, no id — must fold into a leaf AND a host `/32` Map, attached to the
/// WIRE ROOT (the empty parent the wire normalizes to `root`), exactly as a
/// blank-`attach_to` range fallback does.
///
/// The root is IMPLICIT: it is not an exported node, so there is no id to prove
/// and no node to resolve — which is why this emission is topology-independent
/// and why the promoted `/32` is the fallback prefix's SIBLING.
///
/// The store row is exactly `dynamic-range:range-24:192.168.66.4` with parent
/// `Root`/`None`: the live u14 run's row shape, which deferred on every publish.
#[test]
fn a_promoted_host_row_written_under_the_root_sentinel_folds_at_the_wire_root() {
    let mut config = range_test_config(
        "u15-promoted-root-sentinel",
        vec![range_rule("range-24", "192.168.66.0/24", "")],
    );
    config.ip_ranges.ignore_subnets = Vec::new();
    let canonical = range_canonical("site-1", 1_000, 1_000);
    let artifacts = tunnel_artifacts(&config, &canonical);
    write_dynamic_store(
        &config,
        vec![dynamic_circuit(
            "dynamic-range:range-24:192.168.66.4",
            "Root",
            "192.168.66.4",
            32,
        )],
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let promoted = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:range-24:192.168.66.4")
        .expect("the `Root`-sentinel row must fold: it is the shape the actor writes");
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic-range:range-24:192.168.66.4")),
        "the promoted row must not defer: {:?}",
        inputs.warnings
    );
    // PINNED (REV-T3): the sentinel is the wire root, not a node id derived from
    // the fixture — literal empty parent, literal `Root` name.
    assert_eq!(
        promoted.effective_parent_node_name, "Root",
        "the root attachment is the wire's own sentinel name"
    );
    assert!(
        promoted.effective_parent_node_id.is_empty(),
        "the wire root carries no id: {}",
        promoted.effective_parent_node_id
    );
    assert_eq!(
        promoted.logical_parent_node_name.as_deref(),
        Some("Root"),
        "the fold normalizes the row to the sentinel name"
    );
    assert_eq!(
        promoted.logical_parent_node_id, None,
        "the sentinel row carries no parent id"
    );

    // The promoted `/32` is the fallback prefix's SIBLING: same wire-root parent.
    let fallback = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:range-24")
        .expect("the range fallback folds too");
    assert_eq!(
        (fallback.effective_parent_node_id.as_str(), fallback.effective_parent_node_name.as_str()),
        (promoted.effective_parent_node_id.as_str(), promoted.effective_parent_node_name.as_str()),
        "promoted host and fallback prefix are siblings at the wire root"
    );

    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    let leaf_parent = tx.leaf_ops.iter().find_map(|op| match op {
        DataplaneLeafOp::Upsert {
            circuit_id,
            parent_node_id,
            ..
        } if circuit_id == "dynamic-range:range-24:192.168.66.4" => Some(parent_node_id.clone()),
        _ => None,
    });
    assert_eq!(
        leaf_parent.as_deref(),
        Some(""),
        "the promoted leaf hangs off the empty wire-root sentinel: {:?}",
        tx.leaf_ops
    );
    assert_eq!(
        lqos_dataplane_types::normalize_parent_id(&leaf_parent.expect("the leaf was found")),
        lqos_dataplane_types::DATA_PLANE_ROOT_NODE_ID,
        "and the wire normalizes that sentinel to `root`"
    );
    assert!(
        tx.mapping_ops.iter().any(|op| matches!(
            op,
            DataplaneMappingOp::Map { circuit_id, ip, prefix_len: 32, .. }
                if circuit_id == "dynamic-range:range-24:192.168.66.4"
                    && ip.to_string() == "192.168.66.4"
        )),
        "the promoted circuit must emit its /32 Map: {:?}",
        tx.mapping_ops
    );
}

/// u15 A2 + driver ruling (2026-11-17), REV-C1 closure: a FOREST world — several
/// top-level nodes, the shipped `network.example.json` shape and the standard
/// multi-site import — folds the sentinel row too, at the same wire root.
///
/// This replaces the pre-ruling deferral gate. The ruling's reason is structural:
/// the root is implicit, the sentinel resolves to it without consulting the
/// world's shape, so there is nothing for a forest to disagree with. (The
/// pre-ruling code resolved the sentinel to "the effective node with no logical
/// parent" and deferred whenever several existed — promotion then worked only in
/// a single-root world.)
#[test]
fn a_root_sentinel_row_folds_at_the_wire_root_in_a_forest() {
    let mut config = range_test_config(
        "u15-promoted-root-forest",
        vec![range_rule("range-24", "192.168.66.0/24", "")],
    );
    config.ip_ranges.ignore_subnets = Vec::new();
    let canonical = tunnel_canonical(&["site-1", "site-2"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    assert_eq!(
        artifacts
            .effective
            .nodes
            .iter()
            .filter(|node| node.logical_parent_node_id.trim().is_empty())
            .count(),
        2,
        "PREMISE: the world has two top-level nodes — a forest, not a single root"
    );
    write_dynamic_store(
        &config,
        vec![dynamic_circuit(
            "dynamic-range:range-24:192.168.66.8",
            "Root",
            "192.168.66.8",
            32,
        )],
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let promoted = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:range-24:192.168.66.8")
        .expect("the sentinel row folds in a forest: the wire root is topology-independent");
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic-range:range-24:192.168.66.8")),
        "no deferral, no `no single root` warning: {:?}",
        inputs.warnings
    );
    assert!(
        promoted.effective_parent_node_id.is_empty(),
        "the forest world emits the same wire-root sentinel: {}",
        promoted.effective_parent_node_id
    );
    assert_eq!(promoted.effective_parent_node_name, "Root");

    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    let leaf_parent = tx.leaf_ops.iter().find_map(|op| match op {
        DataplaneLeafOp::Upsert {
            circuit_id,
            parent_node_id,
            ..
        } if circuit_id == "dynamic-range:range-24:192.168.66.8" => Some(parent_node_id.clone()),
        _ => None,
    });
    assert_eq!(
        leaf_parent.as_deref(),
        Some(""),
        "the promoted leaf hangs off the wire root in a forest too: {:?}",
        tx.leaf_ops
    );
    assert!(
        tx.mapping_ops.iter().any(|op| matches!(
            op,
            DataplaneMappingOp::Map { circuit_id, ip, prefix_len: 32, .. }
                if circuit_id == "dynamic-range:range-24:192.168.66.8"
                    && ip.to_string() == "192.168.66.8"
        )),
        "the promoted circuit must emit its /32 Map: {:?}",
        tx.mapping_ops
    );
}

/// u15 A2 + driver ruling (2026-11-17), REV-T1 closure: a row with NO parent
/// information at all (blank `parent_node`, no id) is the SAME instruction as the
/// explicit `Root` sentinel — "if no `attach_to` is specified, you attach to the
/// top of the tree" — so both fold at the wire root, identically.
///
/// The two rows are folded in ONE world so the gate proves the predicate is one:
/// identical parent id, name and leaf emission.
#[test]
fn a_blank_parent_row_and_the_root_sentinel_fold_identically() {
    let mut config = range_test_config(
        "u15-promoted-blank-parent",
        vec![range_rule("range-24", "192.168.66.0/24", "")],
    );
    config.ip_ranges.ignore_subnets = Vec::new();
    let canonical = range_canonical("site-1", 1_000, 1_000);
    let artifacts = tunnel_artifacts(&config, &canonical);
    write_dynamic_store(
        &config,
        vec![
            dynamic_circuit("dynamic-range:range-24:192.168.66.4", "Root", "192.168.66.4", 32),
            dynamic_circuit("dynamic-range:range-24:192.168.66.5", "", "192.168.66.5", 32),
        ],
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let named = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:range-24:192.168.66.4")
        .expect("the `Root` row folds");
    let blank = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:range-24:192.168.66.5")
        .expect("the blank-parent row folds: no parent information means the top of the tree");
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic-range:range-24:192.168.66.")),
        "neither row may defer: {:?}",
        inputs.warnings
    );
    assert_eq!(
        (blank.effective_parent_node_id.as_str(), blank.effective_parent_node_name.as_str()),
        (named.effective_parent_node_id.as_str(), named.effective_parent_node_name.as_str()),
        "blank and `Root` are ONE predicate: identical parent id and name"
    );
    assert!(
        blank.effective_parent_node_id.is_empty(),
        "and that shared parent is the wire root sentinel"
    );
    assert_eq!(blank.effective_parent_node_name, "Root");

    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    for expected in [
        "dynamic-range:range-24:192.168.66.4",
        "dynamic-range:range-24:192.168.66.5",
    ] {
        let leaf_parent = tx.leaf_ops.iter().find_map(|op| match op {
            DataplaneLeafOp::Upsert {
                circuit_id,
                parent_node_id,
                ..
            } if circuit_id == expected => Some(parent_node_id.clone()),
            _ => None,
        });
        assert_eq!(
            leaf_parent.as_deref(),
            Some(""),
            "'{expected}' must emit its leaf at the wire root: {:?}",
            tx.leaf_ops
        );
    }
    assert!(
        tx.mapping_ops.iter().any(|op| matches!(
            op,
            DataplaneMappingOp::Map { circuit_id, ip, prefix_len: 32, .. }
                if circuit_id == "dynamic-range:range-24:192.168.66.5"
                    && ip.to_string() == "192.168.66.5"
        )),
        "the blank-parent promoted circuit emits its /32 Map too: {:?}",
        tx.mapping_ops
    );
}

/// u15 driver ruling (2026-11-17), REV-D1 closure: in FLAT compile mode the
/// exported effective network is the generated bucket network, so the pre-ruling
/// root resolution (which read the site/hierarchy state and looked its answer up
/// in the bucket export) could never agree and every sentinel row deferred with a
/// misleading cause.
///
/// With the wire-root emission there is nothing to resolve, so the row folds and
/// is then placed by the ordinary flat-bucket assignment — the same path the
/// range fallback prefix takes. The gate asserts exactly that sameness.
#[test]
fn a_promoted_host_row_folds_like_the_fallback_in_flat_mode() {
    let mut config = range_test_config(
        "u15-promoted-root-flat",
        vec![range_rule("range-24", "192.168.66.0/24", "")],
    );
    config.ip_ranges.ignore_subnets = Vec::new();
    config.topology.compile_mode = "flat".to_string();
    config.queues.override_available_queues = Some(4);
    let canonical = range_canonical("site-1", 1_000, 1_000);
    let artifacts = tunnel_artifacts(&config, &canonical);
    assert!(
        artifacts
            .effective_network
            .as_ref()
            .and_then(|network| network.as_object())
            .is_some_and(|nodes| nodes.keys().all(|name| name.starts_with("Generated_PN_"))),
        "PREMISE: flat mode exports the generated bucket network, not the sites"
    );
    write_dynamic_store(
        &config,
        vec![dynamic_circuit(
            "dynamic-range:range-24:192.168.66.6",
            "Root",
            "192.168.66.6",
            32,
        )],
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic-range:range-24:192.168.66.6")),
        "a flat-mode box must not defer the promoted row: {:?}",
        inputs.warnings
    );
    let promoted = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:range-24:192.168.66.6")
        .expect("the promoted row folds in flat mode");
    let fallback = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:range-24")
        .expect("the fallback prefix folds in flat mode");
    assert!(
        promoted
            .effective_parent_node_id
            .starts_with("libreqos:generated:flat:bucket:"),
        "the ordinary flat-bucket assignment places the promoted row: {}",
        promoted.effective_parent_node_id
    );
    assert!(
        fallback
            .effective_parent_node_id
            .starts_with("libreqos:generated:flat:bucket:"),
        "PREMISE: the fallback prefix is placed by the same assignment: {}",
        fallback.effective_parent_node_id
    );

    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    assert!(
        tx.leaf_ops.iter().any(|op| matches!(
            op,
            DataplaneLeafOp::Upsert { circuit_id, .. }
                if circuit_id == "dynamic-range:range-24:192.168.66.6"
        )),
        "the promoted circuit emits a leaf in flat mode: {:?}",
        tx.leaf_ops
    );
    assert!(
        tx.mapping_ops.iter().any(|op| matches!(
            op,
            DataplaneMappingOp::Map { circuit_id, ip, prefix_len: 32, .. }
                if circuit_id == "dynamic-range:range-24:192.168.66.6"
                    && ip.to_string() == "192.168.66.6"
        )),
        "and its /32 Map: {:?}",
        tx.mapping_ops
    );
}

/// u15 A2, the boundary: the sentinel branch is gated on the row carrying NO id.
/// A row that DOES carry a provable id keeps the id-shaped resolution, so a name
/// that happens to say `Root` cannot outvote it. (A row whose id is NOT provable
/// defers earlier, at the K1 §7 pre-check — `a_promoted_host_row_carrying_an_unprovable_parent_id_defers`.)
#[test]
fn a_root_sentinel_row_that_carries_a_provable_id_resolves_by_that_id() {
    let mut config = range_test_config(
        "u15-promoted-root-with-id",
        vec![range_rule("range-24", "192.168.66.0/24", "")],
    );
    config.ip_ranges.ignore_subnets = Vec::new();
    // A two-level world: `pop1` hangs from the root node `site-1`, so a provable
    // id and the root node are DIFFERENT parents.
    let mut canonical = tunnel_canonical(&["site-1", "pop1"]);
    if let Some(pop) = canonical
        .nodes
        .iter_mut()
        .find(|node| node.node_name == "pop1")
    {
        pop.current_parent_node_id = Some("tunnel-circuits:site:site-1".to_string());
        pop.current_parent_node_name = Some("site-1".to_string());
    }
    let artifacts = tunnel_artifacts(&config, &canonical);
    let mut row = dynamic_circuit(
        "dynamic-range:range-24:192.168.66.9",
        "Root",
        "192.168.66.9",
        32,
    );
    row.shaped.parent_node_id = Some("tunnel-circuits:site:pop1".to_string());
    write_dynamic_store(&config, vec![row]);

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let promoted = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:range-24:192.168.66.9")
        .expect("a provable id must still fold the row");
    assert_eq!(
        promoted.effective_parent_node_id, "tunnel-circuits:site:pop1",
        "an id the export proves is the parent: the `Root` NAME does not outvote it"
    );
}

/// u13: the defect this unit closes. A promoted row with the CORRECT parent name
/// plus an id the exported tree cannot verify — the id the raw `network.json`
/// parser reports for its nodes — DEFERS, which is the live u8 failure
/// `has an unresolvable fallback parent/anchor id`. An unprovable id is not a
/// bonus identity, it is a deferral, so promotion writes the name alone.
#[test]
fn a_promoted_host_row_carrying_an_unprovable_parent_id_defers() {
    let (config, artifacts) = promoted_row_world("u13-promoted-unprovable-id");
    let mut row = dynamic_circuit(
        "dynamic-range:range-24:192.168.66.7",
        "site-1",
        "192.168.66.7",
        32,
    );
    // The raw legacy view's id for that node; the export's id is
    // `tunnel-circuits:site:site-1`, so this cannot resolve.
    row.shaped.parent_node_id = Some("raw-view-site-id".to_string());
    write_dynamic_store(&config, vec![row]);

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    assert!(
        !inputs
            .circuits
            .iter()
            .any(|circuit| circuit.circuit_id == "dynamic-range:range-24:192.168.66.7"),
        "an unprovable parent id must not fold the row: the name alone is what the fold proves"
    );
    assert!(
        inputs.warnings.iter().any(|warning| warning
            .contains("has an unresolvable fallback parent/anchor id")),
        "the id-driven deferral must be named loudly: {:?}",
        inputs.warnings
    );
}

/// u13 A3(c): a promoted row whose parent NAME matches no exported node still
/// DEFERS by name and says so. Dropping the id must not turn an unknown parent
/// into a silent root attachment (an empty parent is the bridge's typed reject).
#[test]
fn a_promoted_host_row_with_an_unknown_parent_name_defers() {
    let (config, artifacts) = promoted_row_world("u13-promoted-unknown-parent");
    write_dynamic_store(
        &config,
        vec![dynamic_circuit(
            "dynamic-range:range-24:192.168.66.6",
            "No Such Site",
            "192.168.66.6",
            32,
        )],
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    assert!(
        !inputs
            .circuits
            .iter()
            .any(|circuit| circuit.circuit_id == "dynamic-range:range-24:192.168.66.6"),
        "an unknown parent name must not fold the row anywhere"
    );
    assert!(
        inputs.warnings.iter().any(|warning| warning.contains(
            "'dynamic-range:range-24:192.168.66.6' has an unresolvable parent 'No Such Site'"
        )),
        "the deferral must name the row and its parent: {:?}",
        inputs.warnings
    );
}

/// One `[[dynamic_circuits.ranges]]` rule, destructured so a test names only
/// what it varies.
fn range_rule(
    name: &str,
    cidr: &str,
    attach_to: &str,
) -> lqos_config::DynamicCircuitRangeRule {
    lqos_config::DynamicCircuitRangeRule {
        name: name.to_string(),
        ip_range: cidr.parse().expect("the rule cidr parses"),
        download_min_mbps: 10.0,
        upload_min_mbps: 5.0,
        download_max_mbps: 100.0,
        upload_max_mbps: 50.0,
        attach_to: attach_to.to_string(),
    }
}

/// One canonical site carrying explicit rates, so a fallback's configured
/// rates are provably NOT clamped by the parent budget.
fn range_canonical(
    site: &str,
    download_mbps: u64,
    upload_mbps: u64,
) -> TopologyCanonicalStateFile {
    let mut canonical = tunnel_canonical(&[site]);
    for node in &mut canonical.nodes {
        node.rate_input = TopologyCanonicalRateInput {
            intrinsic_download_mbps: Some(download_mbps),
            intrinsic_upload_mbps: Some(upload_mbps),
            legacy_imported_download_mbps: Some(download_mbps),
            legacy_imported_upload_mbps: Some(upload_mbps),
            source: TopologyCanonicalRateInputSource::OperatorOverride,
            ..TopologyCanonicalRateInput::default()
        };
    }
    canonical
}

/// A hermetic world carrying `ranges` with the promotion feature ON (the gate
/// the retired compiler branch used).
fn range_test_config(
    prefix: &str,
    ranges: Vec<lqos_config::DynamicCircuitRangeRule>,
) -> Config {
    let mut config = tunnel_test_config(prefix);
    config.dynamic_circuits = Some(lqos_config::DynamicCircuitsConfig {
        enabled: true,
        enable_unknown_ip_promotion: true,
        ranges,
        ..lqos_config::DynamicCircuitsConfig::default()
    });
    config
}

/// The single fallback leaf of `circuit_id` in `tx`.
fn fallback_leaf<'a>(
    tx: &'a lqos_dataplane_types::DataplaneTransaction,
    circuit_id: &str,
) -> &'a lqos_dataplane_types::DataplaneLeafOp {
    tx.leaf_ops
        .iter()
        .find(|op| matches!(
            op,
            lqos_dataplane_types::DataplaneLeafOp::Upsert { circuit_id: leaf, .. }
                if leaf == circuit_id
        ))
        .unwrap_or_else(|| panic!("'{circuit_id}' must emit an ordinary leaf: {:?}", tx.leaf_ops))
}

/// u5 A1/A3/A6: a configured range rule is ONE ORDINARY circuit row — one
/// ordinary device over the rule's prefix at the rule's four rates — compiled
/// through the normal leaf + Map path. Nothing about it is a compiler special
/// case: `compile_dataplane_transaction` sees a shaping-input circuit like any
/// other.
#[test]
fn range_rule_folds_into_one_ordinary_circuit_leaf_and_prefix_map() {
    let config = range_test_config(
        "u5-range-ordinary",
        vec![range_rule("dr-pool", "10.60.0.0/24", "site-1")],
    );
    // A neighbouring ORDINARY circuit (REV-T2): the AQM gate needs a leaf the box
    // default demonstrably reaches, or "the fallback is FIFO" proves nothing.
    fs::write(
        ConfigShapedDevices::path_for_config(&config),
        static_shaped_devices_csv(),
    )
    .expect("the provisioned neighbour csv should write");
    let canonical = range_canonical("site-1", 1_000, 1_000);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");

    let fallback = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:dr-pool")
        .expect("the range rule must fold into an ordinary circuit");
    assert_eq!(
        fallback.devices.len(),
        1,
        "one ordinary device row carries the rule prefix"
    );
    assert_eq!(fallback.devices[0].ipv4, vec!["10.60.0.0/24".to_string()]);
    assert!(
        fallback.devices[0].dynamic,
        "the fallback is dynamic-origin: the u3 supersede rule and the u4 \
         promotion coverage must both see it as synthesized, not provisioned"
    );
    assert_eq!(fallback.effective_parent_node_name, "site-1");
    assert_eq!(
        fallback.effective_parent_node_id,
        "tunnel-circuits:site:site-1",
        "attach_to resolves to the effective node, like any other row"
    );
    assert_eq!(
        (
            fallback.download_min_mbps,
            fallback.upload_min_mbps,
            fallback.download_max_mbps,
            fallback.upload_max_mbps
        ),
        (10.0, 5.0, 100.0, 50.0),
        "the rule's four rates are the plan"
    );
    assert_eq!(
        fallback.sqm_override.as_deref(),
        Some("none"),
        "the retired unconfirmed leaf carried no AQM; the ordinary row pins that explicitly"
    );

    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);

    let leaf = fallback_leaf(&tx, "dynamic-range:dr-pool");
    let lqos_dataplane_types::DataplaneLeafOp::Upsert {
        parent_node_id,
        download,
        upload,
        aqm,
        ..
    } = leaf
    else {
        unreachable!("the helper only returns Upserts");
    };
    assert_eq!(parent_node_id, "tunnel-circuits:site:site-1");
    assert_eq!(
        (download.commit_kbps, download.ceil_kbps),
        (10_000, 100_000),
        "the rule's configured commit/ceil, unclamped by the site"
    );
    assert_eq!((upload.commit_kbps, upload.ceil_kbps), (5_000, 50_000));
    assert_eq!(
        *aqm,
        lqos_dataplane_types::DataplaneLeafAqm::None,
        "the fallback keeps the retired leaf's FIFO shape (Oracle ruling 1)"
    );

    // REV-T2: `compile_dataplane_transaction` passes `default_sqm = "none"`, so the
    // assertion above cannot tell the pin from a leaf that simply inherited the
    // default. Compile the SAME world with the box default a production box
    // configures (`publish.rs` passes `[queues] default_sqm`, typically CAKE): the
    // fallback must stay FIFO while the ordinary neighbour gets CAKE.
    let tx_cake = compile_dataplane_transaction_with_sqm(&network, &inputs, "cake diffserv4");
    let lqos_dataplane_types::DataplaneLeafOp::Upsert { aqm: pinned, .. } =
        fallback_leaf(&tx_cake, "dynamic-range:dr-pool")
    else {
        unreachable!("the helper only returns Upserts");
    };
    assert_eq!(
        *pinned,
        lqos_dataplane_types::DataplaneLeafAqm::None,
        "an explicit `none` override outranks the box default: a dropped or \
         overwritten pin would shape the whole fallback prefix with CAKE"
    );
    let lqos_dataplane_types::DataplaneLeafOp::Upsert {
        aqm: neighbour_aqm,
        ..
    } = fallback_leaf(&tx_cake, "static-1")
    else {
        unreachable!("the helper only returns Upserts");
    };
    assert!(
        matches!(
            neighbour_aqm,
            lqos_dataplane_types::DataplaneLeafAqm::Cake {
                diffserv4: true,
                ..
            }
        ),
        "PREMISE: the box default reaches an ordinary leaf ({neighbour_aqm:?}); \
         without this the pin assertion above proves nothing"
    );

    assert_eq!(
        tx.mapping_ops
            .iter()
            .filter(|op| matches!(
                op,
                lqos_dataplane_types::DataplaneMappingOp::Map { prefix_len: 24, ip, circuit_id, device_id }
                    if ip.to_string() == "10.60.0.0"
                        && circuit_id == "dynamic-range:dr-pool"
                        && device_id == "dynamic-range:dr-pool"
            ))
            .count(),
        1,
        "exactly one prefix Map carries the first-packet coverage: {:?}",
        tx.mapping_ops
    );
    assert!(
        tx.inner_ops.iter().any(|op| matches!(
            op,
            lqos_dataplane_types::DataplaneInnerOp::Upsert { node_id, .. } if node_id == "tunnel-circuits:site:site-1"
        )),
        "the fallback's parent ancestry must materialize: {:?}",
        tx.inner_ops
    );
}

/// u5 A3: the fallback prefix is a longest-prefix entry, so a promoted /32 and
/// a provisioned device inside the range both shadow it — and each keeps its own
/// circuit, leaf and Map (the /24 changes nobody's identity).
#[test]
fn more_specific_circuits_shadow_the_range_fallback() {
    let promoted_ips = ["10.60.0.5", "10.60.0.9", "10.60.0.42"];
    let config = range_test_config(
        "u5-range-shadowing",
        vec![range_rule("dr-pool", "10.60.0.0/24", "site-1")],
    );
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);

    // A PROVISIONED device inside the range: it must keep its own circuit.
    let mut csv = String::from(
        "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
    );
    csv.push_str("\"provisioned-1\",\"Provisioned 1\",\"provisioned-dev\",\"Provisioned 1\",\"site-1\",\"tunnel-circuits:site:site-1\",\"\",\"\",\"10.60.0.77/32\",\"\",\"20\",\"10\",\"200\",\"100\",\"\"\n");
    std::fs::write(ConfigShapedDevices::path_for_config(&config), csv)
        .expect("the provisioned csv should write");
    // Promotions are ordinary store rows.
    write_dynamic_store(
        &config,
        promoted_ips
            .iter()
            .map(|ip| dynamic_circuit(&format!("dynamic-range:dr-pool:{ip}"), "site-1", ip, 32))
            .collect(),
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    assert_eq!(
        inputs
            .circuits
            .iter()
            .filter(|circuit| circuit.circuit_id == "dynamic-range:dr-pool")
            .count(),
        1,
        "exactly one fallback circuit covers the whole range"
    );
    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);

    let mapped = |prefix_len: u8, ip: &str| -> Vec<(String, String)> {
        tx.mapping_ops
            .iter()
            .filter_map(|op| match op {
                lqos_dataplane_types::DataplaneMappingOp::Map {
                    prefix_len: len,
                    ip: mapped,
                    circuit_id,
                    device_id,
                } if *len == prefix_len && mapped.to_string() == ip => {
                    Some((circuit_id.clone(), device_id.clone()))
                }
                _ => None,
            })
            .collect()
    };

    assert_eq!(
        mapped(24, "10.60.0.0"),
        vec![(
            "dynamic-range:dr-pool".to_string(),
            "dynamic-range:dr-pool".to_string()
        )],
        "one fallback Map at the rule prefix"
    );
    for ip in promoted_ips {
        assert_eq!(
            mapped(32, ip),
            vec![(
                format!("dynamic-range:dr-pool:{ip}"),
                format!("dynamic-range:dr-pool:{ip}-dev")
            )],
            "{ip} keeps its own promoted /32 (a strictly longer prefix shadows the /24)"
        );
    }
    assert_eq!(
        mapped(32, "10.60.0.77"),
        vec![
            ("provisioned-1".to_string(), "provisioned-dev".to_string())
        ],
        "a provisioned device inside the range keeps its circuit"
    );
    assert_eq!(
        tx.mapping_ops.len(),
        promoted_ips.len() + 2,
        "conservation: N promoted /32 Maps + the provisioned /32 + the one /24 fallback"
    );
}

/// u5 A5: a `0.0.0.0/0` (and `::/0`) fallback compiles without panic, is its own
/// circuit id, and leaves a more-specific /32 to shadow it. They are ordinary
/// rows, so the root attachment is the wire's `""` sentinel (normalized to
/// `root`), which is exactly the parent the retired branch emitted.
///
/// The world has no ignored subnets: u7 subtracts `ip_ranges.ignore_subnets`
/// from every fallback, so the DEFAULT list would fragment the catch-all and
/// this test's subject is the zero prefix itself (`catch_all_fallback_excludes_
/// ignored_subnets_end_to_end` owns the fragmentation).
#[test]
fn zero_prefix_range_fallbacks_compile_at_the_root() {
    let mut config = range_test_config(
        "u5-range-zero-prefix",
        vec![
            range_rule("default-v4", "0.0.0.0/0", ""),
            range_rule("default-v6", "::/0", "   "),
        ],
    );
    config.ip_ranges.ignore_subnets = Vec::new();
    let canonical = tunnel_canonical(&[]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");

    let v4 = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:default-v4")
        .expect("the v4 catch-all folds");
    assert_eq!(v4.devices[0].ipv4, vec!["0.0.0.0/0".to_string()]);
    assert_eq!(
        v4.devices[0].ipv6,
        Vec::<String>::new(),
        "a v4 rule never lands in the v6 map list"
    );
    assert_eq!(v4.effective_parent_node_name, "Root");
    assert!(
        v4.effective_parent_node_id.is_empty(),
        "blank attach_to is the dataplane root sentinel"
    );
    let v6 = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:default-v6")
        .expect("the v6 catch-all folds");
    assert_eq!(v6.devices[0].ipv6, vec!["::/0".to_string()]);

    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    let lqos_dataplane_types::DataplaneLeafOp::Upsert {
        parent_node_id: v4_parent,
        download,
        ..
    } = fallback_leaf(&tx, "dynamic-range:default-v4")
    else {
        unreachable!("the helper only returns Upserts");
    };
    assert!(
        v4_parent.is_empty(),
        "root attachment is the empty sentinel the wire normalizes to `root`"
    );
    assert_eq!(
        (download.commit_kbps, download.ceil_kbps),
        (10_000, 100_000),
        "the root is unclamped"
    );
    assert!(
        tx.mapping_ops.iter().any(|op| matches!(
            op,
            lqos_dataplane_types::DataplaneMappingOp::Map { prefix_len: 0, ip, circuit_id, .. }
                if ip.to_string() == "0.0.0.0" && circuit_id == "dynamic-range:default-v4"
        )),
        "the v4 default route is its own entry: {:?}",
        tx.mapping_ops
    );
    assert!(
        tx.mapping_ops.iter().any(|op| matches!(
            op,
            lqos_dataplane_types::DataplaneMappingOp::Map { prefix_len: 0, ip, circuit_id, .. }
                if ip.to_string() == "::" && circuit_id == "dynamic-range:default-v6"
        )),
        "the v6 default route is its own entry: {:?}",
        tx.mapping_ops
    );
    assert!(
        tx.inner_ops.is_empty(),
        "the root is implicit: no inner node is created for a catch-all fallback"
    );
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("unresolved in runtime topology")),
        "a blank attach_to is an INTENTIONAL root attachment, not an unresolved \
         parent: the dataplane root is implicit and can never be exported: {:?}",
        inputs.warnings
    );
}

/// u5 A1: an `attach_to` that matches no effective node DEFERS the rule loudly
/// with no fallback circuit at all — the retired branch's behaviour, now in the
/// fold where the rest of the topology decisions live.
#[test]
fn range_rule_with_unresolvable_attach_defers_loudly() {
    let config = range_test_config(
        "u5-range-deferred",
        vec![range_rule("dr-pool", "10.70.0.0/24", "no-such-site")],
    );
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    assert!(
        !inputs
            .circuits
            .iter()
            .any(|circuit| circuit.circuit_id == "dynamic-range:dr-pool"),
        "an unresolvable attach_to must not produce a whole-prefix fallback"
    );
    assert!(
        inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic_range_parent_unresolved")),
        "the deferral must be named loudly: {:?}",
        inputs.warnings
    );
}

/// u5 A1: a range fallback never shadows a provisioned circuit — the id
/// collision is rejected exactly like a colliding RADIUS row.
#[test]
fn range_rule_colliding_with_a_provisioned_circuit_is_rejected() {
    let config = range_test_config(
        "u5-range-collision",
        vec![range_rule("dr-pool", "10.60.0.0/24", "site-1")],
    );
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let mut csv = String::from(
        "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
    );
    csv.push_str("\"dynamic-range:dr-pool\",\"dr-pool\",\"provisioned-dev\",\"Provisioned\",\"site-1\",\"tunnel-circuits:site:site-1\",\"\",\"\",\"10.60.0.9/32\",\"\",\"10\",\"5\",\"100\",\"50\",\"\"\n");
    std::fs::write(ConfigShapedDevices::path_for_config(&config), csv)
        .expect("the provisioned csv should write");

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let circuit = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:dr-pool")
        .expect("the provisioned circuit survives");
    assert_eq!(
        circuit.devices[0].ipv4,
        vec!["10.60.0.9".to_string()],
        "the provisioned device is kept; the fallback row is rejected"
    );
    assert!(
        inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("collides with an assembled circuit id")),
        "the rejection must be named loudly: {:?}",
        inputs.warnings
    );
}

/// u5 A1 / REV-T3: BOTH operands of the gate are load-bearing, and the fold
/// inherits the retired branch's gate (the dynamic feature AND unknown-IP
/// promotion). `enabled` is the product kill switch: a fold that checked only the
/// promotion flag would silently re-enable whole-prefix fallback coverage on
/// boxes that turned the feature off. A ranges-only world has no other shaping
/// source, so it must collapse to `None`; with a provisioned row present the
/// sharper assertion applies — the world exists and carries NO fallback.
#[test]
fn range_rules_are_inert_without_the_feature_or_the_promotion_gate() {
    let world = |config: &Config| -> Option<TopologyShapingInputsFile> {
        let canonical = tunnel_canonical(&["site-1"]);
        let artifacts = tunnel_artifacts(config, &canonical);
        build_shaping_inputs(config, &artifacts).expect("shaping inputs should build")
    };
    let fallbacks = |inputs: &TopologyShapingInputsFile| -> Vec<String> {
        inputs
            .circuits
            .iter()
            .filter(|circuit| circuit.circuit_id.starts_with("dynamic-range:"))
            .map(|circuit| circuit.circuit_id.clone())
            .collect()
    };

    // Operand 1: promotion disabled means no fallback at all.
    let mut promotion_off = range_test_config(
        "u5-range-gate-promotion-off",
        vec![range_rule("dr-pool", "10.60.0.0/24", "site-1")],
    );
    promotion_off
        .dynamic_circuits
        .as_mut()
        .expect("the test config carries the section")
        .enable_unknown_ip_promotion = false;
    assert!(
        world(&promotion_off).is_none(),
        "with promotion off a ranges-only world has no shaping source at all"
    );

    // Operand 2: the feature kill switch, with promotion still ON — so `enabled`
    // is the only operand left to refuse the fallback.
    let mut feature_off = range_test_config(
        "u5-range-gate-feature-off",
        vec![range_rule("dr-pool", "10.60.0.0/24", "site-1")],
    );
    {
        let dynamic_cfg = feature_off
            .dynamic_circuits
            .as_mut()
            .expect("the test config carries the section");
        dynamic_cfg.enabled = false;
        assert!(
            dynamic_cfg.enable_unknown_ip_promotion,
            "PREMISE: promotion stays on, so this world isolates the kill switch"
        );
    }
    assert!(
        world(&feature_off).is_none(),
        "the feature kill switch alone must disable the fallback"
    );
    // ... and with a provisioned row present the world exists, still with no
    // fallback: the switch is not merely "no live source".
    fs::write(
        ConfigShapedDevices::path_for_config(&feature_off),
        static_shaped_devices_csv(),
    )
    .expect("the provisioned csv should write");
    let inputs = world(&feature_off).expect("the provisioned CSV is a live source");
    assert_eq!(
        fallbacks(&inputs),
        Vec::<String>::new(),
        "a disabled feature emits no fallback circuit"
    );

    // A world with NO `[dynamic_circuits]` section at all (an install upgraded
    // from a config that predates the feature).
    let mut no_section = tunnel_test_config("u5-range-gate-no-section");
    no_section.dynamic_circuits = None;
    fs::write(
        ConfigShapedDevices::path_for_config(&no_section),
        static_shaped_devices_csv(),
    )
    .expect("the provisioned csv should write");
    let inputs = world(&no_section).expect("the provisioned CSV is a live source");
    assert_eq!(
        fallbacks(&inputs),
        Vec::<String>::new(),
        "an absent [dynamic_circuits] section emits no fallback circuit"
    );
}

/// u5 A1: the fold rejects a rule it cannot name or one that repeats an accepted
/// rule's identity — no silent whole-prefix circuit either way.
#[test]
fn range_rules_are_rejected_without_a_usable_name_or_a_free_identity() {
    let world = |config: &Config| -> Option<TopologyShapingInputsFile> {
        let canonical = tunnel_canonical(&["site-1"]);
        let artifacts = tunnel_artifacts(config, &canonical);
        build_shaping_inputs(config, &artifacts).expect("shaping inputs should build")
    };

    // A nameless rule warns; a rule whose normalized identity repeats an accepted
    // rule is rejected — one fallback survives, not three.
    let config = range_test_config(
        "u5-range-unusable-names",
        vec![
            range_rule("   ", "10.61.0.0/24", "site-1"),
            range_rule("dr-pool", "10.60.0.0/24", "site-1"),
            range_rule(" DR-POOL ", "10.60.1.0/24", "site-1"),
        ],
    );
    let inputs = world(&config)
        .expect("a configured range is a live shaping source even with no CSV");
    let fallbacks: Vec<&str> = inputs
        .circuits
        .iter()
        .filter(|circuit| circuit.circuit_id.starts_with("dynamic-range:"))
        .map(|circuit| circuit.circuit_id.as_str())
        .collect();
    assert_eq!(
        fallbacks,
        vec!["dynamic-range:dr-pool"],
        "only the first usable rule folds"
    );
    assert!(
        inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("empty name")),
        "the nameless rule must warn: {:?}",
        inputs.warnings
    );
    assert!(
        inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("is claimed by an earlier range rule")),
        "the repeated identity must warn with its own cause: {:?}",
        inputs.warnings
    );
}

/// u5 A1 / REV-C1: a DEFERRED rule must not reserve its circuit id.
///
/// The id is claimed only by a row that was actually pushed, so a rule that defers
/// on an unresolvable `attach_to` cannot deny its identity to a later valid rule —
/// and the collision warning keeps its two causes apart.
#[test]
fn a_deferred_range_rule_does_not_reserve_its_circuit_id() {
    let config = range_test_config(
        "u5-range-deferred-id",
        vec![
            range_rule("dr-pool", "10.70.0.0/24", "no-such-site"),
            range_rule("dr-pool", "10.60.0.0/24", "site-1"),
        ],
    );
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");

    let fallback = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:dr-pool")
        .expect("the second rule keeps the identity the deferred first rule never held");
    assert_eq!(
        fallback.devices[0].ipv4,
        vec!["10.60.0.0/24".to_string()],
        "the ACCEPTED rule is the row that exists"
    );
    assert!(
        inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic_range_parent_unresolved")),
        "the deferral is still reported: {:?}",
        inputs.warnings
    );
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("claimed by an earlier range rule")),
        "a deferred rule never claims an id: {:?}",
        inputs.warnings
    );
}

/// u5 A1 / REV-D1: the range fallback answers to the ORDINARY control surface.
///
/// It is a device row at the RADIUS fold's seam, so the operator override layer
/// that governs every other circuit governs it too: `RemoveCircuit` stops the
/// whole-prefix fallback (leaf AND prefix Map), and `CircuitAdjustSpeed`
/// re-rates it. Before the seam change both were silent no-ops on
/// `dynamic-range:<rule>`.
#[test]
fn range_fallback_honours_circuit_removal_and_rate_adjustments() {
    let write_overrides = |config: &Config, adjustments: serde_json::Value| {
        fs::write(
            PathBuf::from(&config.lqos_directory).join("lqos_overrides.json"),
            serde_json::to_string_pretty(&json!({
                "persistent_devices": [],
                "circuit_adjustments": adjustments,
                "network_adjustments": []
            }))
            .expect("the operator layer serializes"),
        )
        .expect("the operator layer writes");
    };

    // (a) RemoveCircuit.
    let removed = range_test_config(
        "u5-range-override-remove",
        vec![range_rule("dr-pool", "10.60.0.0/24", "site-1")],
    );
    write_overrides(
        &removed,
        json!([{"type": "remove_circuit", "circuit_id": "dynamic-range:dr-pool"}]),
    );
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&removed, &canonical);
    let inputs = build_shaping_inputs(&removed, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    assert!(
        !inputs
            .circuits
            .iter()
            .any(|circuit| circuit.circuit_id == "dynamic-range:dr-pool"),
        "an operator removal must stop the fallback circuit"
    );
    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    assert!(
        !tx.mapping_ops.iter().any(|op| matches!(
            op,
            lqos_dataplane_types::DataplaneMappingOp::Map { circuit_id, .. }
                if circuit_id == "dynamic-range:dr-pool"
        )),
        "a removed fallback must not keep packet-one coverage: {:?}",
        tx.mapping_ops
    );

    // (b) CircuitAdjustSpeed.
    let adjusted = range_test_config(
        "u5-range-override-speed",
        vec![range_rule("dr-pool", "10.60.0.0/24", "site-1")],
    );
    write_overrides(
        &adjusted,
        json!([{
            "type": "circuit_adjust_speed",
            "circuit_id": "dynamic-range:dr-pool",
            "min_download_bandwidth": 2.0,
            "max_download_bandwidth": 40.0,
            "min_upload_bandwidth": 1.0,
            "max_upload_bandwidth": 20.0
        }]),
    );
    let canonical = range_canonical("site-1", 1_000, 1_000);
    let artifacts = tunnel_artifacts(&adjusted, &canonical);
    let inputs = build_shaping_inputs(&adjusted, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let fallback = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dynamic-range:dr-pool")
        .expect("the fallback survives a rate adjustment");
    assert_eq!(
        (
            fallback.download_min_mbps,
            fallback.upload_min_mbps,
            fallback.download_max_mbps,
            fallback.upload_max_mbps
        ),
        (2.0, 1.0, 40.0, 20.0),
        "the operator's rates replace the rule's plan, exactly like any circuit"
    );
}

/// u5 A1 / REV-D1 (second half): a rule prefix inside `ip_ranges.ignore_subnets`
/// never reaches `mapping_ops`, and the refusal is NAMED.
///
/// The device loop prunes ignored addresses; a rule whose whole prefix is ignored
/// is the same condition one level up. Silence here would leave an operator with a
/// configured catch-all and no coverage at all.
#[test]
fn range_rule_inside_an_ignored_subnet_never_reaches_mapping_ops() {
    let mut config = range_test_config(
        "u5-range-ignored-subnet",
        vec![
            range_rule("dr-pool", "10.60.0.0/24", "site-1"),
            // A rule OUTSIDE the ignored set in the same world, so the refusal is
            // provably per prefix and not a whole-feature switch.
            range_rule("dr-other", "10.61.0.0/24", "site-1"),
        ],
    );
    config.ip_ranges.ignore_subnets = vec!["10.60.0.0/16".to_string()];
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    assert!(
        !inputs
            .circuits
            .iter()
            .any(|circuit| circuit.circuit_id == "dynamic-range:dr-pool"),
        "an ignored subnet must not become a fallback circuit"
    );
    assert!(
        inputs
            .circuits
            .iter()
            .any(|circuit| circuit.circuit_id == "dynamic-range:dr-other"),
        "an un-ignored rule in the same world still folds"
    );
    assert!(
        inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic_range_ignored_subnet")),
        "the refusal must be named: {:?}",
        inputs.warnings
    );
    let network = artifacts
        .effective_network
        .clone()
        .expect("the effective network must exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    assert!(
        !tx.mapping_ops.iter().any(|op| matches!(
            op,
            lqos_dataplane_types::DataplaneMappingOp::Map { circuit_id, .. }
                if circuit_id == "dynamic-range:dr-pool"
        )),
        "no ignored prefix may reach mapping_ops: {:?}",
        tx.mapping_ops
    );
}

/// The root-attachment branch that the range fold needs (repair round 1, REV-D1)
/// is shared, so it is gated on an ORDINARY provisioned row too: `Root` is
/// implicit in the effective network, and a row naming it is attached to the
/// dataplane root — not an unresolved parent, and not a row whose display name is
/// lost. The compiled parent is the same empty sentinel the wire normalizes.
#[test]
fn a_provisioned_row_naming_the_root_attaches_to_the_dataplane_root() {
    let config = tunnel_test_config("u5-root-parent-provided");
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let mut csv = String::from(
        "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
    );
    csv.push_str("\"rooted-1\",\"Rooted 1\",\"rooted-dev\",\"Rooted 1\",\"Root\",\"\",\"\",\"\",\"203.0.113.20/32\",\"\",\"10\",\"5\",\"100\",\"50\",\"\"\n");
    fs::write(ConfigShapedDevices::path_for_config(&config), csv)
        .expect("the provisioned csv should write");

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let circuit = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "rooted-1")
        .expect("the provisioned row must fold");
    assert_eq!(
        circuit.resolution_source,
        lqos_config::TopologyShapingResolutionSource::LegacyParent,
        "the root is a resolution TARGET, not a fallback"
    );
    assert!(
        circuit.effective_parent_node_id.is_empty(),
        "the root's parent id is the wire's empty sentinel"
    );
    assert_eq!(
        circuit.effective_parent_node_name, "Root",
        "the display parent is the store's root node, so the circuit renders attached"
    );
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("unresolved in runtime topology")),
        "a deliberate root attachment is not an unresolved parent: {:?}",
        inputs.warnings
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

/// K1 §7 (the coordinator rev 594): a fallback-supplied parent/anchor id set to the
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

/// One provisioned row in the manual `ShapedDevices.csv` the fold reads.
fn static_shaped_devices_csv() -> &'static str {
    concat!(
        "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,",
        "Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,",
        "Download Max Mbps,Upload Max Mbps,Comment\n",
        "\"static-1\",\"Static 1\",\"static-dev-1\",\"Static 1\",\"site-1\",\"\",",
        "\"\",\"aa:bb:cc:dd:ee:00\",\"203.0.113.10/32\",\"\",10,5,100,50,\"\"\n",
    )
}

/// Design card §3 rule 2: the fold marks the rows IT synthesized, and only
/// those. A provisioned row from the same artifact stays unmarked, so the
/// supersede rule can tell provisioning from an overlay row.
#[test]
fn k1_dynamic_fold_marks_only_synthesized_rows_as_dynamic_origin() {
    let config = tunnel_test_config("k1-dynamic-origin-marker");
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    fs::write(
        PathBuf::from(&config.lqos_directory).join("ShapedDevices.csv"),
        static_shaped_devices_csv(),
    )
    .expect("ShapedDevices.csv should write");
    write_dynamic_store(
        &config,
        vec![dynamic_circuit("dyn-1", "site-1", "10.0.0.5", 32)],
    );

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let static_circuit = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "static-1")
        .expect("the provisioned circuit must be present");
    assert!(
        !static_circuit.devices[0].dynamic,
        "a provisioned row is never marked dynamic-origin"
    );
    let dynamic_circuit = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dyn-1")
        .expect("the dynamic circuit must be present");
    assert!(
        dynamic_circuit.devices[0].dynamic,
        "the fold must mark the row it synthesized"
    );
    assert_eq!(
        dynamic_circuit.devices.len(),
        1,
        "the marker is per synthesized device row"
    );
}

/// A1: the marker must not churn artifacts that carry no dynamic rows. A
/// static-only `shaping_inputs.json` writes no `dynamic` key, and a payload an
/// earlier build wrote (no such field) loads as static and re-serializes to the
/// same bytes.
#[test]
fn static_only_shaping_inputs_serialize_without_the_origin_marker() {
    let config = tunnel_test_config("k1-dynamic-origin-bytes");
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    fs::write(
        PathBuf::from(&config.lqos_directory).join("ShapedDevices.csv"),
        static_shaped_devices_csv(),
    )
    .expect("ShapedDevices.csv should write");

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let bytes = serde_json::to_string(&inputs).expect("shaping inputs serialize");
    assert!(
        !bytes.contains("\"dynamic\""),
        "a static-only artifact must not gain the marker key: {bytes}"
    );

    // The shape an earlier build wrote: the same bytes, without the field.
    let reloaded: TopologyShapingInputsFile =
        serde_json::from_str(&bytes).expect("the pre-marker artifact still parses");
    assert_eq!(reloaded, inputs, "the round-trip preserves the payload");
    assert!(
        reloaded.circuits.iter().all(|circuit| circuit
            .devices
            .iter()
            .all(|device| !device.dynamic)),
        "the absent field reads as a provisioned row"
    );
    assert_eq!(
        serde_json::to_string(&reloaded).expect("the reloaded payload serializes"),
        bytes,
        "re-serializing a static artifact must reproduce it byte for byte"
    );
}

/// A5: the marker is topology-layer only. The same topology compiles to the
/// same transaction AND the same content key with and without it, so no unit
/// changes the wire.
#[test]
fn origin_marker_never_reaches_the_compiled_transaction() {
    let network = json!({
        "site-a": {
            "id": "site-a-id",
            "name": "site-a",
            "downloadBandwidthMbps": 1000,
            "uploadBandwidthMbps": 1000,
            "children": {}
        }
    });
    let circuit = TopologyShapingCircuitInput {
        circuit_id: "dyn-1".to_string(),
        circuit_name: "Dynamic 1".to_string(),
        effective_parent_node_name: "site-a".to_string(),
        effective_parent_node_id: "site-a-id".to_string(),
        download_min_mbps: 1.0,
        upload_min_mbps: 1.0,
        download_max_mbps: 10.0,
        upload_max_mbps: 10.0,
        devices: vec![TopologyShapingDeviceInput {
            device_id: "dyn-dev-1".to_string(),
            device_name: "Dynamic 1".to_string(),
            ipv4: vec!["100.64.0.5/32".to_string()],
            ..TopologyShapingDeviceInput::default()
        }],
        ..TopologyShapingCircuitInput::default()
    };
    let provisioned = TopologyShapingInputsFile {
        circuits: vec![circuit.clone()],
        ..TopologyShapingInputsFile::default()
    };
    let marked = TopologyShapingInputsFile {
        circuits: vec![TopologyShapingCircuitInput {
            devices: vec![TopologyShapingDeviceInput {
                dynamic: true,
                ..circuit.devices[0].clone()
            }],
            ..circuit
        }],
        ..TopologyShapingInputsFile::default()
    };

    let provisioned_tx = compile_dataplane_transaction(&network, &provisioned);
    let marked_tx = compile_dataplane_transaction(&network, &marked);
    assert_eq!(
        provisioned_tx, marked_tx,
        "the origin marker must not change a single dataplane op"
    );
    assert_eq!(
        lqos_dataplane_types::content_key(&provisioned_tx),
        lqos_dataplane_types::content_key(&marked_tx),
        "the content key is the wire identity and must be identical"
    );
}

/// A persistent device override PROVISIONS the circuit it names, so it must not
/// inherit the dynamic origin: an operator who pins a live RADIUS circuit as a
/// persistent device has provisioned it, and the supersede rule must be able to
/// retire the dynamic membership. The override replaces the folded row in place
/// (same `device_id`), so the marker is decided on the row the operator supplied.
#[test]
fn persistent_override_provisioning_clears_the_dynamic_origin() {
    let config = tunnel_test_config("k1-dynamic-origin-override");
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    write_dynamic_store(
        &config,
        vec![dynamic_circuit("dyn-1", "site-1", "10.0.0.5", 32)],
    );
    fs::write(
        PathBuf::from(&config.lqos_directory).join("lqos_overrides.json"),
        serde_json::to_string_pretty(&json!({
            "persistent_devices": [{
                "circuit_id": "dyn-1",
                "circuit_name": "Dynamic 1 (provisioned)",
                "device_id": "dyn-1-dev",
                "device_name": "Dynamic 1 (provisioned)",
                "parent_node": "site-1",
                "parent_node_id": null,
                "anchor_node_id": null,
                "mac": "",
                "ipv4": [["10.0.0.5", 32]],
                "ipv6": [],
                "download_min_mbps": 20.0,
                "upload_min_mbps": 10.0,
                "download_max_mbps": 200.0,
                "upload_max_mbps": 100.0,
                "comment": "provisioned by the operator",
                "sqm_override": null
            }],
            "circuit_adjustments": [],
            "network_adjustments": []
        }))
        .expect("override json should serialize"),
    )
    .expect("override file should write");

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let circuit = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dyn-1")
        .expect("the provisioned circuit must be present");
    assert_eq!(circuit.devices.len(), 1, "the override replaces the row");
    assert_eq!(
        circuit.devices[0].comment, "provisioned by the operator",
        "the surviving row is the operator's persistent device"
    );
    assert!(
        !circuit.devices[0].dynamic,
        "a persistent override provisions the circuit; it cannot stay dynamic-origin"
    );
}

/// REV-D1 regression gate: the APPEND override shape.
///
/// A persistent device override matches by `device_id`: with the circuit's
/// `circuit_id` but a different `device_id` it APPENDS a second row and the
/// fold's row stays in the world. The origin is keyed per ROW, so the fold row
/// must stay marked (it is still synthesized) while the appended operator row is
/// provisioned. Keying by circuit id cleared the whole circuit and re-armed
/// self-supersession for the fold row.
#[test]
fn a_persistent_override_appending_a_row_keeps_the_fold_rows_origin() {
    let config = tunnel_test_config("k1-dynamic-origin-override-append");
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    write_dynamic_store(
        &config,
        vec![dynamic_circuit("dyn-1", "site-1", "10.0.0.5", 32)],
    );
    fs::write(
        PathBuf::from(&config.lqos_directory).join("lqos_overrides.json"),
        serde_json::to_string_pretty(&json!({
            "persistent_devices": [{
                "circuit_id": "dyn-1",
                "circuit_name": "Dynamic 1 (pinned)",
                "device_id": "dyn-1-extra",
                "device_name": "Dynamic 1 (pinned)",
                "parent_node": "site-1",
                "parent_node_id": null,
                "anchor_node_id": null,
                "mac": "",
                "ipv4": [["10.0.0.6", 32]],
                "ipv6": [],
                "download_min_mbps": 20.0,
                "upload_min_mbps": 10.0,
                "download_max_mbps": 200.0,
                "upload_max_mbps": 100.0,
                "comment": "appended by the operator",
                "sqm_override": null
            }],
            "circuit_adjustments": [],
            "network_adjustments": []
        }))
        .expect("override json should serialize"),
    )
    .expect("override file should write");

    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should be present");
    let circuit = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dyn-1")
        .expect("the circuit carries both rows");
    assert_eq!(
        circuit.devices.len(),
        2,
        "the override appends; it does not replace"
    );

    let fold_row = circuit
        .devices
        .iter()
        .find(|device| device.device_id == "dyn-1-dev")
        .expect("the fold's own row is still in the world");
    assert!(
        fold_row.dynamic,
        "the fold row is still synthesized: an appended override row must not clear it"
    );
    let appended_row = circuit
        .devices
        .iter()
        .find(|device| device.device_id == "dyn-1-extra")
        .expect("the override row is appended");
    assert!(
        !appended_row.dynamic,
        "the operator's row is provisioned, not dynamic-origin"
    );
}

/// REV-T5: the byte-identity gate against a COMMITTED literal payload.
///
/// The earlier gate compared this build's serializer against itself, so a field
/// rename would have left it green while breaking every `shaping_inputs.json`
/// written by an earlier release. This literal is the pre-marker shape: no
/// `dynamic` key anywhere.
#[test]
fn a_committed_pre_marker_payload_loads_and_reserializes_byte_identically() {
    const PRE_MARKER: &str = r#"{"schema_version":1,"shaping_generation":"abc123","warnings":[],"circuits":[{"circuit_id":"static-1","circuit_name":"Static 1","effective_parent_node_name":"site-1","effective_parent_node_id":"site-1","resolution_source":"topology_anchor","download_min_mbps":10.0,"upload_min_mbps":5.0,"download_max_mbps":100.0,"upload_max_mbps":50.0,"comment":"","devices":[{"device_id":"static-dev-1","device_name":"Static 1","mac":"","ipv4":["203.0.113.10/32"],"ipv6":[],"comment":""}]}]}"#;
    assert!(
        !PRE_MARKER.contains("\"dynamic\""),
        "the committed payload predates the origin marker"
    );

    let parsed: TopologyShapingInputsFile =
        serde_json::from_str(PRE_MARKER).expect("a pre-marker payload must still parse");
    let device = &parsed.circuits[0].devices[0];
    assert!(
        !device.dynamic,
        "an absent marker reads as a provisioned row"
    );
    assert_eq!(
        serde_json::to_string(&parsed).expect("the parsed payload serializes"),
        PRE_MARKER,
        "a field renamed on either side breaks this: the artifact format is a contract"
    );
}

/// REV-C2: a pre-marker artifact is recognized as the same world, so the custody
/// log can name the upgrade instead of reporting a tamper-shaped mismatch.
///
/// The marker stays part of the custody DECISION on purpose: an unmarked world
/// would read every folded row as provisioned and prune every live dynamic
/// circuit on the next reload.
#[test]
fn a_pre_marker_artifact_is_recognized_as_the_same_world() {
    let marked = TopologyShapingInputsFile {
        circuits: vec![TopologyShapingCircuitInput {
            circuit_id: "dyn-1".to_string(),
            circuit_name: "Dynamic 1".to_string(),
            effective_parent_node_name: "site-1".to_string(),
            effective_parent_node_id: "site-1".to_string(),
            devices: vec![TopologyShapingDeviceInput {
                device_id: "dyn-1-dev".to_string(),
                device_name: "Dynamic 1".to_string(),
                ipv4: vec!["10.0.0.5/32".to_string()],
                dynamic: true,
                ..TopologyShapingDeviceInput::default()
            }],
            ..TopologyShapingCircuitInput::default()
        }],
        shaping_generation: "generation-1".to_string(),
        ..TopologyShapingInputsFile::default()
    };
    let mut pre_marker = marked.clone();
    pre_marker.circuits[0].devices[0].dynamic = false;

    assert!(
        same_world_ignoring_origin_marker(&pre_marker, &marked),
        "the pre-marker payload is the same world"
    );
    assert!(
        !marked.semantic_equals(&pre_marker),
        "PREMISE: the marker is part of the custody comparison"
    );

    let mut different_world = pre_marker.clone();
    different_world.circuits[0].devices[0].ipv4 = vec!["10.0.0.7/32".to_string()];
    assert!(
        !same_world_ignoring_origin_marker(&different_world, &marked),
        "a content change is not an upgrade artifact"
    );
}

/// u6 A1 (REV-T1): the deferral line is emitted by the SHIPPED publish path,
/// once per world.
///
/// The claim is about what an operator sees, so this gate PUBLISHES and reads
/// the log: five publishes of one world must log exactly ONE deferral WARN
/// (publishes 2..5 read publish 1's list off the artifact) and the artifact on
/// disk must carry that warning. Reading through the shipped
/// `publish_warnings_to_log` also pins the path constant the block depends on:
/// deleting the block silences publish 1, reading the artifact after the dedupe
/// write silences it too, and reading the wrong path re-logs every publish.
///
/// The same world emits ZERO transaction ops for the circuit — an op with an
/// absent parent is exactly what the bridge rejects as "added circuit's parent
/// does not exist in the applied tree" — and the first publish after the parent
/// appears folds it: the deferral defers, it never drops.
#[test]
fn a_deferred_parent_logs_one_warning_through_the_publisher_and_folds_later() {
    let config = tunnel_test_config("u6-deferred-once");
    write_dynamic_store(
        &config,
        vec![dynamic_circuit("dyn-orphan", "no-such-site", "10.0.0.7", 32)],
    );

    // World 1: `no-such-site` does not exist, so every mint defers the row.
    let canonical = tunnel_canonical(&["site-1"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let network = artifacts
        .effective_network
        .clone()
        .expect("effective network should exist");
    let mut deferral_warns: Vec<String> = Vec::new();
    for publish in 1..=5 {
        let warnings = WarningCapture::capturing(|| {
            publish_effective_topology_artifacts(&config, &artifacts)
                .expect("every publish of one world should succeed");
        });
        let deferral_lines: Vec<String> = warnings
            .into_iter()
            .filter(|warning| warning.contains("shaping_deferred"))
            .collect();
        assert_eq!(
            deferral_lines.len(),
            usize::from(publish == 1),
            "publish {publish}: the deferral is news once, then the artifact is the memory: {deferral_lines:?}"
        );
        deferral_warns.extend(deferral_lines);

        // What was PUBLISHED (not a re-mint) is what the bridge would consume.
        let inputs = load_shaping_inputs_at_path(&topology_shaping_inputs_path(&config))
            .expect("the publish writes the shaping inputs it just logged");
        assert!(
            !inputs
                .circuits
                .iter()
                .any(|circuit| circuit.circuit_id == "dyn-orphan"),
            "publish {publish}: a deferred circuit must not fold"
        );
        let tx = compile_dataplane_transaction(&network, &inputs);
        assert!(
            !tx.leaf_ops.iter().any(|op| matches!(
                op,
                DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "dyn-orphan"
            )),
            "publish {publish}: no leaf may carry an absent parent: {:?}",
            tx.leaf_ops
        );
        assert!(
            !tx.mapping_ops.iter().any(|op| matches!(
                op,
                DataplaneMappingOp::Map { circuit_id, .. } if circuit_id == "dyn-orphan"
            )),
            "publish {publish}: no Map op may carry an absent circuit"
        );
    }
    assert_eq!(
        deferral_warns.len(),
        1,
        "five publishes of one world warn once, not per publish: {deferral_warns:?}"
    );
    let on_disk = load_shaping_inputs_at_path(&topology_shaping_inputs_path(&config))
        .expect("the publish writes the shaping inputs");
    assert!(
        on_disk
            .warnings
            .iter()
            .any(|warning| warning.contains("shaping_deferred")),
        "the artifact IS the memory the next publish reads: {:?}",
        on_disk.warnings
    );

    // World 2: the parent appears. The very next publish folds the circuit.
    let canonical = tunnel_canonical(&["site-1", "no-such-site"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let warnings = WarningCapture::capturing(|| {
        publish_effective_topology_artifacts(&config, &artifacts)
            .expect("the resolved world should publish");
    });
    assert!(
        !warnings
            .iter()
            .any(|warning| warning.contains("shaping_deferred")),
        "a resolved world defers nothing: {warnings:?}"
    );
    let network = artifacts
        .effective_network
        .clone()
        .expect("effective network should exist");
    let inputs = load_shaping_inputs_at_path(&topology_shaping_inputs_path(&config))
        .expect("the publish writes the shaping inputs");
    let circuit = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "dyn-orphan")
        .expect("a resolvable parent must fold the circuit");
    assert_eq!(
        circuit.effective_parent_node_id, "tunnel-circuits:site:no-such-site",
        "the parent must resolve through the effective tree"
    );
    let tx = compile_dataplane_transaction(&network, &inputs);
    assert!(
        tx.leaf_ops.iter().any(|op| matches!(
            op,
            DataplaneLeafOp::Upsert { circuit_id, .. } if circuit_id == "dyn-orphan"
        )),
        "the resolved world emits the circuit: {:?}",
        tx.leaf_ops
    );
    assert!(
        !inputs
            .warnings
            .iter()
            .any(|warning| warning.contains("shaping_deferred")),
        "no deferral may remain once the parent resolves: {:?}",
        inputs.warnings
    );
}

/// u6 A1 (REV-T3): every branch of the publish-warning diff, not just the
/// happy path the publish gate happens to walk.
///
/// The doc block promises intra-publish dedupe and "a condition that clears and
/// returns warns again"; both live in branches a single-warning fixture never
/// reaches.
#[test]
fn publish_warning_diff_dedupes_and_rewarns_after_a_clear() {
    let a = "a".to_string();
    let b = "b".to_string();
    let just_a = std::slice::from_ref(&a);
    let duplicated = [a.clone(), a.clone()];

    assert_eq!(
        super::new_publish_warnings(&[], &duplicated),
        vec![a.clone()],
        "duplicates inside one publish are emitted once"
    );
    assert_eq!(
        super::new_publish_warnings(just_a, &[a.clone(), b.clone()]),
        vec![b.clone()],
        "only the line the artifact did not carry is news"
    );
    assert!(
        super::new_publish_warnings(just_a, &[]).is_empty(),
        "a cleared condition logs nothing"
    );
    assert_eq!(
        super::new_publish_warnings(&[], just_a),
        vec![a.clone()],
        "a world that never warned before is news"
    );
}
