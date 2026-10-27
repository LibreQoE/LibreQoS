// T5 rung 1 (t5_topology_manager_consolidation_derivation.md): the
// E2 operator-view builder. One model, two projections — E1 (the
// dataplane transaction) stays the frozen bridge wire with the F-LM
// subtree elision; E2 keeps the FULL topology including circuit-less
// sites (the F-LM §8-4 asymmetry resolved as two honest projections)
// and carries devices inline on circuit nodes (F10: identity without
// ShapedDevices.csv). Determinism is load-bearing: the emitted node
// list is sorted by id and no map iteration order reaches the file.

use lqos_config::{
    deduped_display_names, validate_topology_nodes, OperatorRateSource, OperatorViewDevice,
    OperatorViewNode, TopologyOperatorViewFile,
};
use lqos_overrides::OverrideFile;

/// Rung 1.5 §1.5: the rate-source marker for a site node.
fn site_rate_source(
    overrides: Option<&OverrideFile>,
    node_id: &str,
    node_name: &str,
) -> OperatorRateSource {
    let Some(overrides) = overrides else {
        return OperatorRateSource::Base;
    };
    let adjusted = overrides.network_adjustments().iter().any(|adjustment| match adjustment {
        NetworkAdjustment::AdjustSiteSpeed {
            node_id: id,
            site_name,
            ..
        } => id.as_deref() == Some(node_id) || site_name == node_name,
        _ => false,
    });
    if adjusted {
        OperatorRateSource::Override
    } else {
        OperatorRateSource::Base
    }
}

/// Rung 1.5 §1.5: the rate-source marker for a circuit node.
fn circuit_rate_source(overrides: Option<&OverrideFile>, circuit_id: &str) -> OperatorRateSource {
    let Some(overrides) = overrides else {
        return OperatorRateSource::Base;
    };
    match overrides.circuit_speed_override(circuit_id) {
        Some(speeds) if speeds.is_suspend_floor() => OperatorRateSource::SuspendSentinel,
        Some(_) => OperatorRateSource::Override,
        None => OperatorRateSource::Base,
    }
}

/// Node-type label used by the operator view (matches the store/tree
/// convention the T1 builder and UI consume).
fn normalized_operator_node_type(kind: &str) -> String {
    let lower = kind.trim().to_ascii_lowercase();
    match lower.as_str() {
        "site" => "site".to_string(),
        "ap" => "ap".to_string(),
        "circuit" => "circuit".to_string(),
        "" => "site".to_string(),
        other => other.to_string(),
    }
}

/// One effective-network dict node's rates: `(ceil_dl, ceil_ul,
/// commit_dl, commit_ul)` in Mbps.
type EffectiveRates = (Option<f64>, Option<f64>, Option<f64>, Option<f64>);

fn read_rate(node: &serde_json::Map<String, Value>, key: &str) -> Option<f64> {
    node.get(key).and_then(Value::as_f64)
}

fn collect_effective_rates(
    node: &Value,
    key: Option<&str>,
    rates_by_id: &mut BTreeMap<String, EffectiveRates>,
) {
    let Some(object) = node.as_object() else {
        return;
    };
    if let Some(id) = object.get("id").and_then(Value::as_str).filter(|id| !id.trim().is_empty()) {
        {
            rates_by_id.insert(
                id.to_string(),
                (
                    read_rate(object, "downloadBandwidthMbps"),
                    read_rate(object, "uploadBandwidthMbps"),
                    read_rate(object, "downloadBandwidthMbpsMin"),
                    read_rate(object, "uploadBandwidthMbpsMin"),
                ),
            );
        }
    }
    let _ = key;
    if let Some(children) = object.get("children").and_then(Value::as_object) {
        for (child_key, child) in children {
            collect_effective_rates(child, Some(child_key), rates_by_id);
        }
    }
}

fn canonical_fallback_rates(node: &TopologyCanonicalNode) -> EffectiveRates {
    let as_f64 = |value: Option<u64>| value.map(|v| v as f64);
    let dl = as_f64(node.rate_input.intrinsic_download_mbps);
    let ul = as_f64(node.rate_input.intrinsic_upload_mbps);
    let legacy_dl = as_f64(node.rate_input.legacy_imported_download_mbps);
    let legacy_ul = as_f64(node.rate_input.legacy_imported_upload_mbps);
    (dl.or(legacy_dl), ul.or(legacy_ul), None, None)
}

/// Builds the E2 operator view from the canonical model, the runtime
/// shaping inputs (circuits + inline devices), and the augmented
/// effective network (rate source; in flat mode it is the bucket
/// dict, whose nodes are folded in the same way).
pub fn build_operator_view(
    canonical: &TopologyCanonicalStateFile,
    shaping_inputs: Option<&TopologyShapingInputsFile>,
    effective_network: Option<&Value>,
    overrides: Option<&OverrideFile>,
    source_generation: &str,
) -> Result<TopologyOperatorViewFile, Vec<String>> {
    // Ruling 6: the model validates BEFORE any projection is built.
    validate_topology_nodes(&canonical.nodes)?;

    let (names_by_id, ambiguous) = deduped_display_names(&canonical.nodes);
    let mut rates_by_id: BTreeMap<String, EffectiveRates> = BTreeMap::new();
    if let Some(map) = effective_network.and_then(Value::as_object) {
        for (_key, node) in map {
            collect_effective_rates(node, None, &mut rates_by_id);
        }
    }

    // §lqos-74t.26 (`docs/operator_view_attachment_device_dedup_card.md`): a
    // standalone node that is ONLY an attachment elsewhere (an attachment-only
    // radio), with no children and no direct circuits, must NOT be emitted as a
    // top-level node — the canonical already omits it; the operator view
    // duplicated it. Keep the attachment representation (on the served site).
    let mut attachment_only: BTreeSet<&str> = BTreeSet::new();
    for node in &canonical.nodes {
        for parent in &node.allowed_parents {
            for option in &parent.attachment_options {
                if option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID {
                    attachment_only.insert(option.attachment_id.as_str());
                }
                if !option.attachment_name.trim().is_empty() {
                    attachment_only.insert(option.attachment_name.as_str());
                }
            }
        }
    }
    let mut has_circuits: BTreeSet<&str> = BTreeSet::new();
    if let Some(inputs) = shaping_inputs {
        for circuit in &inputs.circuits {
            has_circuits.insert(circuit.effective_parent_node_id.as_str());
        }
    }

    let mut nodes: Vec<OperatorViewNode> = Vec::new();
    let mut seen_ids: BTreeMap<String, ()> = BTreeMap::new();

    for node in &canonical.nodes {
        if node.node_id.trim().is_empty() || seen_ids.contains_key(&node.node_id) {
            continue;
        }
        // `docs/topology_ptp_link_site_attachment_work_order.md`: a DEVICE node
        // with an EMPTY kind is an ATTACHMENT-ONLY link (its `node_kind` has no
        // `network.json` source) — it must NOT be emitted as a node and NEVER
        // fabricated as a `site` (`normalized_operator_node_type("") => "site"`).
        // It exists only as the site's `current_attachment_id`. Mark it seen so
        // the effective-dict fold does not re-add it.
        if node.node_id.starts_with("uisp:device:") && node.node_kind.trim().is_empty() {
            seen_ids.insert(node.node_id.clone(), ());
            continue;
        }
        let rates = rates_by_id
            .get(node.node_id.as_str())
            .copied()
            .unwrap_or_else(|| canonical_fallback_rates(node));
        seen_ids.insert(node.node_id.clone(), ());
        nodes.push(OperatorViewNode {
            id: node.node_id.clone(),
            name: names_by_id
                .get(node.node_id.as_str())
                .cloned()
                .unwrap_or_else(|| node.node_name.clone()),
            parent_id: node
                .current_parent_node_id
                .clone()
                .filter(|parent| !parent.trim().is_empty()),
            node_type: normalized_operator_node_type(&node.node_kind),
            download_mbps: rates.0,
            upload_mbps: rates.1,
            commit_download_mbps: rates.2,
            commit_upload_mbps: rates.3,
            devices: Vec::new(),
            latitude: node.latitude.map(f64::from),
            longitude: node.longitude.map(f64::from),
            tunnel_identity: None,
            rate_source: site_rate_source(overrides, &node.node_id, &node.node_name),
        });
    }

    // Effective-dict nodes missing from canonical (flat-mode
    // generated buckets) fold in with their dict identity.
    if let Some(map) = effective_network.and_then(Value::as_object) {
        for (key, node) in map {
            collect_missing_effective_nodes(node, key, &rates_by_id, &mut nodes, &mut seen_ids);
        }
    }

    // Circuit nodes from the shaping inputs — devices INLINE (ruling 2).
    if let Some(shaping_inputs) = shaping_inputs {
        for circuit in &shaping_inputs.circuits {
            if circuit.circuit_id.trim().is_empty() || seen_ids.contains_key(&circuit.circuit_id) {
                continue;
            }
            seen_ids.insert(circuit.circuit_id.clone(), ());
            // Rung 1.5 §1.5: a suspended circuit renders the SENTINEL marker,
            // never the broker floor (0.01/0/1024) as a numeric rate.
            let rate_source = circuit_rate_source(overrides, &circuit.circuit_id);
            let suspended = matches!(rate_source, OperatorRateSource::SuspendSentinel);
            let devices = circuit
                .devices
                .iter()
                .map(|device| OperatorViewDevice {
                    device_id: device.device_id.clone(),
                    device_name: device.device_name.clone(),
                    ipv4: device.ipv4.clone(),
                    ipv6: device.ipv6.clone(),
                    mac: device.mac.clone(),
                    download_min_mbps: if suspended {
                        0
                    } else {
                        circuit.download_min_mbps.max(0.0) as u64
                    },
                    upload_min_mbps: if suspended {
                        0
                    } else {
                        circuit.upload_min_mbps.max(0.0) as u64
                    },
                    download_max_mbps: if suspended {
                        0
                    } else {
                        circuit.download_max_mbps.max(0.0) as u64
                    },
                    upload_max_mbps: if suspended {
                        0
                    } else {
                        circuit.upload_max_mbps.max(0.0) as u64
                    },
                })
                .collect();
            nodes.push(OperatorViewNode {
                id: circuit.circuit_id.clone(),
                name: if circuit.circuit_name.trim().is_empty() {
                    circuit.circuit_id.clone()
                } else {
                    circuit.circuit_name.clone()
                },
                parent_id: if circuit.effective_parent_node_id.trim().is_empty() {
                    None
                } else {
                    Some(circuit.effective_parent_node_id.clone())
                },
                node_type: "circuit".to_string(),
                download_mbps: (!suspended).then(|| circuit.download_max_mbps.max(0.0) as f64),
                upload_mbps: (!suspended).then(|| circuit.upload_max_mbps.max(0.0) as f64),
                commit_download_mbps: (!suspended)
                    .then(|| circuit.download_min_mbps.max(0.0) as f64),
                commit_upload_mbps: (!suspended).then(|| circuit.upload_min_mbps.max(0.0) as f64),
                devices,
                latitude: None,
                longitude: None,
                tunnel_identity: circuit.tunnel_identity,
                rate_source,
            });
        }
    }

    // §lqos-74t.26 POST-PASS (re-opened): the canonical loop alone is
    // insufficient — `collect_missing_effective_nodes` folds effective-dict nodes
    // that are NOT canonical nodes (the observed duplicate top-level `ap` nodes:
    // a `uisp:device` id that IS an attachment option but not a canonical node).
    // Re-apply the predicate over the ASSEMBLED vector (canonical + effective +
    // circuit folds): omit a standalone node iff attachment-only (id OR name) AND
    // childless AND no direct circuits. The attachment representation stays.
    let assembled_children: BTreeSet<String> = nodes
        .iter()
        .filter_map(|node| node.parent_id.clone())
        .collect();
    let assembled_circuit_parents: BTreeSet<String> = nodes
        .iter()
        .filter(|node| node.is_circuit())
        .filter_map(|node| node.parent_id.clone())
        .collect();
    let assembled_device_parents: BTreeSet<String> = nodes
        .iter()
        .filter(|node| !node.devices.is_empty())
        .map(|node| node.id.clone())
        .collect();
    nodes.retain(|node| {
        let is_attachment_only = attachment_only.contains(node.id.as_str())
            || attachment_only.contains(node.name.as_str());
        if !is_attachment_only {
            return true;
        }
        let has_child = assembled_children.contains(node.id.as_str());
        let has_direct_circuit = has_circuits.contains(node.id.as_str())
            || assembled_circuit_parents.contains(node.id.as_str())
            || assembled_device_parents.contains(node.id.as_str());
        has_child || has_direct_circuit
    });

    // Determinism pin: id-sorted flat list, built not iterated.
    nodes.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(TopologyOperatorViewFile {
        schema_version: 1,
        source_generation: source_generation.to_string(),
        nodes,
        ambiguous_base_names: ambiguous,
    })
}

fn collect_missing_effective_nodes(
    node: &Value,
    key: &str,
    rates_by_id: &BTreeMap<String, EffectiveRates>,
    nodes: &mut Vec<OperatorViewNode>,
    seen_ids: &mut BTreeMap<String, ()>,
) {
    let Some(object) = node.as_object() else {
        return;
    };
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string);
    if let Some(id) = id.filter(|id| !seen_ids.contains_key(id.as_str())) {
        seen_ids.insert(id.clone(), ());
 // the implementer `#12` R4: effective-dict nodes missing from canonical fold in
        // here. A DEVICE with an EMPTY `type` is an attachment-only link
        // re-entering the view — it must NOT be fabricated as a `site` (this is
        // the `normalized_operator_node_type("") => "site"` home on the
        // effective-dict path). Mark it seen, do not emit, keep walking children.
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !(id.starts_with("uisp:device:") && kind.trim().is_empty()) {
            let rates = rates_by_id.get(id.as_str()).copied();
            nodes.push(OperatorViewNode {
                name: object
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| key.to_string()),
                parent_id: None,
                node_type: normalized_operator_node_type(kind),
                download_mbps: rates.and_then(|rates| rates.0),
                upload_mbps: rates.and_then(|rates| rates.1),
                commit_download_mbps: rates.and_then(|rates| rates.2),
                commit_upload_mbps: rates.and_then(|rates| rates.3),
                devices: Vec::new(),
                latitude: object.get("latitude").and_then(Value::as_f64),
                longitude: object.get("longitude").and_then(Value::as_f64),
                tunnel_identity: None,
                rate_source: OperatorRateSource::Base,
                id,
            });
        }
    }
    if let Some(children) = object.get("children").and_then(Value::as_object) {
        for (child_key, child) in children {
            collect_missing_effective_nodes(child, child_key, rates_by_id, nodes, seen_ids);
        }
    }
}

#[cfg(test)]
mod operator_view_tests {
    use super::*;
    use lqos_config::{TopologyCanonicalNode, TopologyCanonicalStateFile};

    /// `docs/topology_ptp_link_site_attachment_work_order.md`: a DEVICE node with
    /// an EMPTY kind is an ATTACHMENT-ONLY PTP link — it must NOT be emitted as
    /// a node and NEVER fabricated as a `site` (the `"" -> "site"` mapping).
    #[test]
    fn an_empty_kind_device_is_not_emitted_as_a_site() {
        let mut canonical = TopologyCanonicalStateFile::default();
        canonical.nodes = vec![
            TopologyCanonicalNode {
                node_id: "uisp:site:root".to_string(),
                node_name: "Root".to_string(),
                node_kind: "site".to_string(),
                ..Default::default()
            },
            TopologyCanonicalNode {
                node_id: "uisp:device:ap".to_string(),
                node_name: "Serving AP".to_string(),
                node_kind: "ap".to_string(),
                current_parent_node_id: Some("uisp:site:root".to_string()),
                ..Default::default()
            },
            // The ATTACHMENT-ONLY link: a DEVICE with an EMPTY kind.
            TopologyCanonicalNode {
                node_id: "uisp:device:link".to_string(),
                node_name: "Wave Long-Range".to_string(),
                node_kind: String::new(),
                current_parent_node_id: Some("uisp:device:ap".to_string()),
                current_attachment_id: Some("uisp:device:ap".to_string()),
                ..Default::default()
            },
            TopologyCanonicalNode {
                node_id: "uisp:site:child".to_string(),
                node_name: "TrailerCity Tower Trailer".to_string(),
                node_kind: "site".to_string(),
                current_parent_node_id: Some("uisp:device:ap".to_string()),
                current_attachment_id: Some("uisp:device:link".to_string()),
                ..Default::default()
            },
        ];
        let view = build_operator_view(&canonical, None, None, None, "gen-1").expect("view builds");
        assert!(
            !view.nodes.iter().any(|node| node.id == "uisp:device:link"),
            "the empty-kind link is NOT a node (never fabricated as `site`)"
        );
        assert!(view.nodes.iter().any(|node| node.id == "uisp:device:ap"));
        let child = view
            .nodes
            .iter()
            .find(|node| node.id == "uisp:site:child")
            .expect("the child site is a node");
        assert_eq!(child.parent_id.as_deref(), Some("uisp:device:ap"));
    }

 /// the implementer `#12` R4: the operator view folds in effective-dict nodes MISSING
    /// from canonical (`collect_missing_effective_nodes`). An effective-dict
    /// DEVICE with an EMPTY `type` (an attachment-only link) must NOT re-enter
    /// the view fabricated as a `site`.
    #[test]
    fn an_empty_kind_device_in_the_effective_dict_is_not_emitted_as_a_site() {
        let canonical = TopologyCanonicalStateFile {
            nodes: vec![
                TopologyCanonicalNode {
                    node_id: "uisp:site:root".to_string(),
                    node_name: "Root".to_string(),
                    node_kind: "site".to_string(),
                    ..Default::default()
                },
                TopologyCanonicalNode {
                    node_id: "uisp:device:ap".to_string(),
                    node_name: "Serving AP".to_string(),
                    node_kind: "ap".to_string(),
                    current_parent_node_id: Some("uisp:site:root".to_string()),
                    ..Default::default()
                },
                TopologyCanonicalNode {
                    node_id: "uisp:site:child".to_string(),
                    node_name: "TrailerCity Tower Trailer".to_string(),
                    node_kind: "site".to_string(),
                    current_parent_node_id: Some("uisp:device:ap".to_string()),
                    current_attachment_id: Some("uisp:device:link".to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        // The effective network still carries the attachment-only link (empty
        // `type`) — the canonical does not.
        let effective = serde_json::json!({
            "Root": {
                "id": "uisp:site:root",
                "type": "Site",
                "name": "Root",
                "children": {
                    "AP": {
                        "id": "uisp:device:ap",
                        "type": "AP",
                        "name": "Serving AP",
                        "children": {
                            "link": {
                                "id": "uisp:device:link",
                                "type": "",
                                "name": "Wave Long-Range",
                                "children": {}
                            }
                        }
                    }
                }
            }
        });
        let view = build_operator_view(&canonical, None, Some(&effective), None, "gen-1")
            .expect("view builds");
        assert!(
            !view.nodes.iter().any(|node| node.id == "uisp:device:link"),
            "the effective-dict link must NOT re-enter as a fabricated `site`"
        );
        let child = view
            .nodes
            .iter()
            .find(|node| node.id == "uisp:site:child")
            .expect("the child site is a node");
        assert_eq!(child.parent_id.as_deref(), Some("uisp:device:ap"));
    }

    /// §`lqos-74t.26`: an ATTACHMENT-ONLY device (referenced as another node's
    /// attachment option), with no children and no direct circuits, is NOT
    /// emitted as a standalone node — the canonical already omits it. A control
    /// that HOSTS a child site stays.
    #[test]
    fn attachment_only_childless_circuitless_device_is_not_emitted() {
        let attachment: lqos_config::TopologyAttachmentOption =
            serde_json::from_value(serde_json::json!({
                "attachment_id": "uisp:device:radio-1",
                "attachment_name": "LTU-2049BoyScout"
            }))
            .expect("attachment option");
        let canonical = TopologyCanonicalStateFile {
            nodes: vec![
                TopologyCanonicalNode {
                    node_id: "uisp:site:root".to_string(),
                    node_name: "Root".to_string(),
                    node_kind: "site".to_string(),
                    ..Default::default()
                },
                TopologyCanonicalNode {
                    node_id: "uisp:site:boy-scout".to_string(),
                    node_name: "2049 Boy Scout".to_string(),
                    node_kind: "site".to_string(),
                    current_parent_node_id: Some("uisp:site:root".to_string()),
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "uisp:site:root".to_string(),
                        parent_node_name: "Root".to_string(),
                        attachment_options: vec![attachment],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                // The DUPLICATE standalone radio: `ap` kind, no parent, no
                // children, no direct circuits.
                TopologyCanonicalNode {
                    node_id: "uisp:device:radio-1".to_string(),
                    node_name: "LTU-2049BoyScout".to_string(),
                    node_kind: "ap".to_string(),
                    ..Default::default()
                },
                // CONTROL: an attachment-only radio that HOSTS a child ⇒ stays.
                TopologyCanonicalNode {
                    node_id: "uisp:device:radio-2".to_string(),
                    node_name: "AF5XHD-2800AirportRd".to_string(),
                    node_kind: "ap".to_string(),
                    ..Default::default()
                },
                TopologyCanonicalNode {
                    node_id: "uisp:site:under-radio-2".to_string(),
                    node_name: "Under Radio 2".to_string(),
                    node_kind: "site".to_string(),
                    current_parent_node_id: Some("uisp:device:radio-2".to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let view = build_operator_view(&canonical, None, None, None, "gen-1").expect("view builds");
        assert!(
            !view.nodes.iter().any(|node| node.id == "uisp:device:radio-1"),
            "an attachment-only childless circuit-less device is NOT a standalone node"
        );
        assert!(
            view.nodes.iter().any(|node| node.id == "uisp:device:radio-2"),
            "an attachment-only device that HOSTS a child site stays"
        );
    }

    /// §`lqos-74t.26`: a duplicate whose NODE ID differs from the attachment id
    /// but whose NAME matches the attachment name is STILL omitted (the predicate
    /// matches id OR name).
    #[test]
    fn attachment_only_by_name_is_omitted_too() {
        let attachment: lqos_config::TopologyAttachmentOption =
            serde_json::from_value(serde_json::json!({
                "attachment_id": "uisp:device:radio-1",
                "attachment_name": "LTU-2049BoyScout"
            }))
            .expect("attachment option");
        let canonical = TopologyCanonicalStateFile {
            nodes: vec![
                TopologyCanonicalNode {
                    node_id: "uisp:site:root".to_string(),
                    node_name: "Root".to_string(),
                    node_kind: "site".to_string(),
                    ..Default::default()
                },
                TopologyCanonicalNode {
                    node_id: "uisp:site:boy-scout".to_string(),
                    node_name: "2049 Boy Scout".to_string(),
                    node_kind: "site".to_string(),
                    current_parent_node_id: Some("uisp:site:root".to_string()),
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "uisp:site:root".to_string(),
                        parent_node_name: "Root".to_string(),
                        attachment_options: vec![attachment],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                // The duplicate: a DIFFERENT id, the SAME name.
                TopologyCanonicalNode {
                    node_id: "uisp:device:radio-name-only".to_string(),
                    node_name: "LTU-2049BoyScout".to_string(),
                    node_kind: "ap".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let view = build_operator_view(&canonical, None, None, None, "gen-1").expect("view builds");
        assert!(
            !view.nodes
                .iter()
                .any(|node| node.id == "uisp:device:radio-name-only"),
            "an attachment-only node matched by NAME is omitted even when its id differs"
        );
    }

    /// §`lqos-74t.26` POST-PASS (re-opened): the duplicate is emitted by the
    /// EFFECTIVE-dict fold (`collect_missing_effective_nodes`) — it is NOT a
    /// canonical node, only an attachment option. The post-pass must remove it.
    #[test]
    fn effective_only_attachment_node_is_omitted_by_the_post_pass() {
        let attachment: lqos_config::TopologyAttachmentOption =
            serde_json::from_value(serde_json::json!({
                "attachment_id": "uisp:device:radio-1",
                "attachment_name": "LTU-2049BoyScout"
            }))
            .expect("attachment option");
        let canonical = TopologyCanonicalStateFile {
            nodes: vec![
                TopologyCanonicalNode {
                    node_id: "uisp:site:root".to_string(),
                    node_name: "Root".to_string(),
                    node_kind: "site".to_string(),
                    ..Default::default()
                },
                TopologyCanonicalNode {
                    node_id: "uisp:site:boy-scout".to_string(),
                    node_name: "2049 Boy Scout".to_string(),
                    node_kind: "site".to_string(),
                    current_parent_node_id: Some("uisp:site:root".to_string()),
                    allowed_parents: vec![TopologyAllowedParent {
                        parent_node_id: "uisp:site:root".to_string(),
                        parent_node_name: "Root".to_string(),
                        attachment_options: vec![attachment],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        // The effective dict carries the attachment-only radio as an `ap` under
        // root — it is NOT a canonical node.
        let effective = serde_json::json!({
            "Root": {
                "id": "uisp:site:root",
                "type": "Site",
                "name": "Root",
                "children": {
                    "Radio": {
                        "id": "uisp:device:radio-1",
                        "type": "AP",
                        "name": "LTU-2049BoyScout",
                        "children": {}
                    }
                }
            }
        });
        let view = build_operator_view(&canonical, None, Some(&effective), None, "gen-1")
            .expect("view builds");
        assert!(
            !view
                .nodes
                .iter()
                .any(|node| node.id == "uisp:device:radio-1"),
            "an effective-only attachment-only node is removed by the post-pass"
        );
    }
}
