//! lqos-74t.150 full-mode split: a native-canonical-only v2 bundle must
//! export a compatibility tree so its anchors resolve.

use lqos_topology_compile::{TopologyCompileMode, compile_topology, parse_libreqos_network};
use serde_json::Value;
use std::collections::BTreeMap;

fn fixture() -> lqos_topology_compile::ImportedTopologyBundle {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/libreqos_network.json"
    ))
    .expect("v2 fixture readable");
    parse_libreqos_network(&raw)
        .expect("v2 fixture validates")
        .into_imported_bundle("libreqos_network")
        .expect("v2 fixture projects")
}

fn collect_nodes(
    value: &Value,
    parent: Option<&str>,
    nodes: &mut BTreeMap<String, Option<String>>,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string);
    if id.is_none() {
        // The top level is a map of root node names, not a node object.
        // (test-only helper; same fix as 6871de482)
        for child in object.values() {
            collect_nodes(child, parent, nodes);
        }
        return;
    }
    if let Some(id) = id.as_deref() {
        nodes.insert(id.to_string(), parent.map(str::to_string));
    }
    if let Some(children) = object.get("children").and_then(Value::as_object) {
        for child in children.values() {
            collect_nodes(child, id.as_deref(), nodes);
        }
    }
}

#[test]
fn full_mode_synthesizes_the_compatibility_tree_for_a_native_only_v2_bundle() {
    let compiled = compile_topology(fixture(), TopologyCompileMode::Full)
        .expect("full mode must export a native-canonical-only v2 bundle");
    assert!(
        compiled
            .compatibility_network_json
            .as_object()
            .is_some_and(|map| !map.is_empty()),
        "full mode must synthesize the compatibility tree when the v2 importer left it empty"
    );
    assert!(!compiled.circuit_anchors.anchors.is_empty());
    assert!(
        !compiled.canonical.nodes.is_empty(),
        "the canonical projection must survive"
    );

    // The original failure mode: anchors resolved against an empty tree. Walk
    // the synthesized shape, assert every anchor id exists, and assert every
    // non-root parent id is itself in the tree.
    let mut nodes = BTreeMap::new();
    collect_nodes(&compiled.compatibility_network_json, None, &mut nodes);
    for anchor in &compiled.circuit_anchors.anchors {
        let parent = nodes.get(&anchor.anchor_node_id).unwrap_or_else(|| {
            panic!(
                "anchor '{}' points at missing node '{}'",
                anchor.circuit_id, anchor.anchor_node_id
            )
        });
        if let Some(parent_id) = parent {
            assert!(
                nodes.contains_key(parent_id),
                "anchor node '{}' references missing parent '{}'",
                anchor.anchor_node_id,
                parent_id
            );
        }
    }
}
