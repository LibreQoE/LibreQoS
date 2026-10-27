fn count_node_ids(value: &Value, counts: &mut HashMap<String, usize>) {
    let Some(node) = value.as_object() else {
        return;
    };
    if let Some(id) = node.get("id").and_then(Value::as_str) {
        *counts.entry(id.to_string()).or_insert(0) += 1;
    }
    if let Some(children) = node.get("children").and_then(Value::as_object) {
        for child in children.values() {
            count_node_ids(child, counts);
        }
    }
}

fn effective_site_parent_map(
    site_node_ids: &HashSet<&str>,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> HashMap<String, String> {
    let effective_by_node = effective
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut parents = HashMap::new();

    for node in &ui_state.nodes {
        if !site_node_ids.contains(node.node_id.as_str()) {
            continue;
        }
        let selected_parent = effective_by_node
            .get(node.node_id.as_str())
            .map(|entry| entry.logical_parent_node_id.as_str())
            .filter(|parent_id| !parent_id.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| node.current_parent_node_id.clone());
        let Some(parent_id) = selected_parent else {
            continue;
        };
        if !site_node_ids.contains(parent_id.as_str()) {
            continue;
        }
        parents.insert(node.node_id.clone(), parent_id);
    }

    parents
}

fn validate_effective_site_parent_cycles(
    site_node_ids: &HashSet<&str>,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    errors: &mut Vec<String>,
) {
    let parents = effective_site_parent_map(site_node_ids, ui_state, effective);
    for site_id in parents.keys() {
        let mut seen = HashSet::new();
        let mut cursor = site_id.as_str();
        while let Some(parent_id) = parents.get(cursor) {
            if !seen.insert(cursor.to_string()) {
                let node_name = ui_state
                    .find_node(site_id)
                    .map(|node| node.node_name.clone())
                    .unwrap_or_else(|| site_id.clone());
                errors.push(format!(
                    "Effective topology would create a parent cycle involving '{}'.",
                    node_name
                ));
                break;
            }
            cursor = parent_id.as_str();
        }
    }
}

fn canonical_site_node_ids(canonical: &TopologyCanonicalStateFile) -> HashSet<&str> {
    canonical
        .nodes
        .iter()
        .filter(|node| node.node_kind.eq_ignore_ascii_case("site"))
        .map(|node| node.node_id.as_str())
        .collect()
}

fn validate_effective_node_identity_consistency(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    errors: &mut Vec<String>,
) {
    let mut ui_counts = HashMap::<&str, usize>::new();
    for node in &ui_state.nodes {
        *ui_counts.entry(node.node_id.as_str()).or_default() += 1;
    }
    for (node_id, count) in ui_counts {
        if count > 1 {
            errors.push(format!(
                "Canonical topology editor state contains duplicate node id '{}'.",
                node_id
            ));
        }
    }

    let mut effective_counts = HashMap::<&str, usize>::new();
    for node in &effective.nodes {
        *effective_counts.entry(node.node_id.as_str()).or_default() += 1;
    }
    for (node_id, count) in effective_counts {
        if count > 1 {
            errors.push(format!(
                "Effective topology state contains duplicate node id '{}'.",
                node_id
            ));
        }
    }

    for canonical_node in &ui_state.nodes {
        if !effective
            .nodes
            .iter()
            .any(|effective_node| effective_node.node_id == canonical_node.node_id)
        {
            errors.push(format!(
                "Effective topology state is missing node '{}'.",
                canonical_node.node_name
            ));
        }
    }

    for node in &effective.nodes {
        let Some(ui_node) = ui_state.find_node(&node.node_id) else {
            errors.push(format!(
                "Effective topology state references unknown node id '{}'.",
                node.node_id
            ));
            continue;
        };

        if node.logical_parent_node_id.is_empty() {
            if node.effective_attachment_id.is_some() {
                errors.push(format!(
                    "Effective topology selected attachment for '{}' without a logical parent.",
                    ui_node.node_name
                ));
            }
            continue;
        }

        let Some(selected_parent) = ui_node
            .allowed_parents
            .iter()
            .find(|parent| parent.parent_node_id == node.logical_parent_node_id)
        else {
            let fixed_attachment_id = ui_node
                .current_attachment_id
                .as_deref()
                .filter(|attachment_id| !attachment_id.is_empty());
            let legacy_fixed_parent = ui_node.allowed_parents.is_empty()
                && ui_node.current_parent_node_id.as_deref()
                    == Some(node.logical_parent_node_id.as_str())
                && node.preferred_attachment_id.as_deref() == fixed_attachment_id
                && node.effective_attachment_id.as_deref() == fixed_attachment_id;
            if legacy_fixed_parent {
                continue;
            }
            errors.push(format!(
                "Effective topology selected invalid parent '{}' for '{}'.",
                node.logical_parent_node_id, ui_node.node_name
            ));
            continue;
        };

        if let Some(preferred_attachment_id) = node.preferred_attachment_id.as_deref()
            && !selected_parent
                .attachment_options
                .iter()
                .any(|option| option.attachment_id == preferred_attachment_id)
        {
            errors.push(format!(
                "Effective topology selected invalid preferred attachment '{}' for '{}'.",
                preferred_attachment_id, ui_node.node_name
            ));
        }

        if let Some(effective_attachment_id) = node.effective_attachment_id.as_deref()
            && !selected_parent
                .attachment_options
                .iter()
                .any(|option| option.attachment_id == effective_attachment_id)
        {
            errors.push(format!(
                "Effective topology selected invalid attachment '{}' for '{}'.",
                effective_attachment_id, ui_node.node_name
            ));
        }
    }
}

/// Validates that the candidate effective tree is structurally safe to publish.
///
/// This checks that the effective topology remains ID-consistent, the effective
/// site-parent graph is acyclic, and every canonical site node remains present
/// exactly once in the exported tree.
fn validate_effective_topology_network_from_canonical(
    _config: &Config,
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    effective_network: &Value,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let site_node_ids = canonical_site_node_ids(canonical);
    validate_effective_node_identity_consistency(ui_state, effective, &mut errors);
    validate_effective_site_parent_cycles(&site_node_ids, ui_state, effective, &mut errors);

    let mut counts = HashMap::new();
    let Some(root) = effective_network.as_object() else {
        return Err(vec![
            "Effective topology export is not a JSON object tree.".to_string(),
        ]);
    };
    for child in root.values() {
        count_node_ids(child, &mut counts);
    }

    for canonical_node in canonical
        .nodes
        .iter()
        .filter(|node| site_node_ids.contains(node.node_id.as_str()))
    {
        match counts
            .get(&canonical_node.node_id)
            .copied()
            .unwrap_or_default()
        {
            1 => {}
            0 => errors.push(format!(
                "Effective topology export dropped site '{}'.",
                canonical_node.node_name
            )),
            count => errors.push(format!(
                "Effective topology export duplicated site '{}' {} times.",
                canonical_node.node_name, count
            )),
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validates that an effective topology export remains structurally safe to publish.
///
/// This legacy helper accepts the candidate canonical network tree directly and reconstructs
/// canonical topology metadata from it so existing call sites and tests can keep using the same
/// interface.
pub fn validate_effective_topology_network(
    config: &Config,
    canonical_network: &Value,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    effective_network: &Value,
) -> Result<(), Vec<String>> {
    let canonical_state = TopologyCanonicalStateFile::from_editor_and_network(
        ui_state,
        canonical_network,
        TopologyCanonicalIngressKind::NativeIntegration,
    );
    validate_effective_topology_network_from_canonical(
        config,
        &canonical_state,
        ui_state,
        effective,
        effective_network,
    )
}

/// Applies the effective attachment selection to a canonical network tree and returns
/// the runtime-effective tree used by shaping/export.
fn apply_effective_topology_to_network_json_from_canonical(
    config: &Config,
    canonical_network: &Value,
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    direct_circuit_hosts: &DirectCircuitHosts,
    read_only: bool,
) -> std::result::Result<Value, Vec<String>> {
    let mut out =
        apply_effective_topology_reparenting_only(canonical_network, ui_state, effective)?;
    if let Some(root) = out.as_object_mut() {
        recompile_effective_network_bandwidths(root, canonical, ui_state, effective);
        // K1b §4.1 item 1: fold the MERGED `AdjustSiteSpeed` overrides
        // (operator + stormguard, operator-first) onto the effective node
        // ceils AFTER the canonical/attachment recomputation so a StormGuard
        // clamp reaches the compiled inner op (and the broker delta).
        let runtime_overrides = load_runtime_shaping_overrides(config, read_only)
            .map_err(|err| vec![format!("{err:#}")])?;
        apply_site_speed_overrides(root, runtime_overrides.network_adjustments());
        apply_runtime_squashing(config, ui_state, effective, direct_circuit_hosts, root)
            .map_err(|err| vec![err])?;
    }
    Ok(out)
}

/// §`lqos-74t.29`: remove top-level attachment-only nodes from the compiled
/// network tree (the `tree.html?parent=0` root duplication). A node is removed
/// iff attachment-only (id OR name is an attachment option) AND childless AND
/// NOT a direct circuit host (`circuit_parent_ids`) — the effective network IS
/// the DPDK compiler input, so a circuit host must stay (else the leaf-parent
/// chain breaks -> UnknownParent -> R3).
fn remove_top_level_attachment_only_nodes(
    out: &mut Value,
    canonical: &TopologyCanonicalStateFile,
    circuit_parent_ids: &BTreeSet<&str>,
) {
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
    let Some(root) = out.as_object_mut() else {
        return;
    };
    root.retain(|key, value| {
        let object = value.as_object();
        let id = object
            .and_then(|o| o.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let name = object
            .and_then(|o| o.get("name"))
            .and_then(Value::as_str)
            .unwrap_or(key.as_str());
        let is_attachment_only =
            attachment_only.contains(id) || attachment_only.contains(name);
        if !is_attachment_only {
            return true;
        }
        // GUARD: a circuit host must stay (the effective network is the compiler
        // input; removing it would orphan its leaf).
        if circuit_parent_ids.contains(id) {
            return true;
        }
        let has_children = object
            .and_then(|o| o.get("children"))
            .and_then(Value::as_object)
            .map(|children| !children.is_empty())
            .unwrap_or(false);
        has_children
    });
}

/// Applies the effective attachment selection to a canonical network tree and returns
/// the runtime-effective tree used by shaping/export.
///
/// Returns errors instead of a partial tree when reparenting cannot find the target
/// parent or would overwrite an existing child subtree.
///
/// This legacy helper accepts the candidate canonical network tree directly and reconstructs
/// canonical topology metadata from it so existing call sites and tests can keep using the same
/// interface.
pub fn apply_effective_topology_to_network_json(
    config: &Config,
    canonical_network: &Value,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> std::result::Result<Value, Vec<String>> {
    let canonical_state = TopologyCanonicalStateFile::from_editor_and_network(
        ui_state,
        canonical_network,
        TopologyCanonicalIngressKind::NativeIntegration,
    );
    apply_effective_topology_to_network_json_from_canonical(
        config,
        canonical_network,
        &canonical_state,
        ui_state,
        effective,
        &DirectCircuitHosts::default(),
        false,
    )
}

fn apply_effective_topology_to_canonical_state(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    direct_circuit_hosts: &DirectCircuitHosts,
    read_only: bool,
) -> std::result::Result<Value, Vec<String>> {
    let canonical_network =
        if canonical.ingress_kind == TopologyCanonicalIngressKind::NativeIntegration {
            canonical.insight_topology_network_json()
        } else {
            canonical.compatibility_network_json().clone()
        };
    apply_effective_topology_to_network_json_from_canonical(
        config,
        &canonical_network,
        canonical,
        ui_state,
        effective,
        direct_circuit_hosts,
        read_only,
    )
}

#[cfg(test)]
mod lqos74t29_tests {
    use super::*;

    /// §`lqos-74t.29`: a top-level attachment-only childless, circuit-less node is
    /// removed; a CIRCUIT HOST stays even when childless; a child-hosting node
    /// stays.
    #[test]
    fn top_level_attachment_only_removed_but_circuit_host_kept() {
        let option = |id: &str, name: &str| -> lqos_config::TopologyAttachmentOption {
            serde_json::from_value(serde_json::json!({
                "attachment_id": id,
                "attachment_name": name
            }))
            .expect("attachment option")
        };
        let canonical = TopologyCanonicalStateFile {
            nodes: vec![TopologyCanonicalNode {
                node_id: "uisp:site:one".to_string(),
                node_name: "Site One".to_string(),
                node_kind: "site".to_string(),
                allowed_parents: vec![TopologyAllowedParent {
                    parent_node_id: "root".to_string(),
                    parent_node_name: "Root".to_string(),
                    attachment_options: vec![
                        option("uisp:device:radio-1", "LTU-2049BoyScout"),
                        option("uisp:device:radio-2", "AF5XHD-2800AirportRd"),
                        option("uisp:device:radio-3", "Wave-Parent"),
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut out = serde_json::json!({
            "Site One": { "id": "uisp:site:one", "name": "Site One", "children": {} },
            "LTU-2049BoyScout": { "id": "uisp:device:radio-1", "name": "LTU-2049BoyScout", "children": {} },
            "AF5XHD-2800AirportRd": { "id": "uisp:device:radio-2", "name": "AF5XHD-2800AirportRd", "children": {} },
            "Wave-Parent": { "id": "uisp:device:radio-3", "name": "Wave-Parent",
                "children": { "Sub": { "id": "uisp:site:sub", "name": "Sub", "children": {} } } }
        });
        // radio-2 is a CIRCUIT HOST (its id is a circuit's effective parent).
        let circuit_parents: BTreeSet<&str> = ["uisp:device:radio-2"].into_iter().collect();
        remove_top_level_attachment_only_nodes(&mut out, &canonical, &circuit_parents);
        let root = out.as_object().expect("object");
        assert!(
            !root.contains_key("LTU-2049BoyScout"),
            "an attachment-only childless circuit-less top-level node is removed"
        );
        assert!(
            root.contains_key("AF5XHD-2800AirportRd"),
            "a CIRCUIT HOST stays even when childless"
        );
        assert!(
            root.contains_key("Wave-Parent"),
            "an attachment-only node that HOSTS children stays"
        );
        assert!(root.contains_key("Site One"));
    }

    /// `lqos-74t.15.2` (A1) — the `lqos-74t.29` no-op is SUPERSEDED: a childless,
    /// circuit-less top-level node IS LOAD-BEARING (`analysis.rs:186` +
    /// `walk_place:347`) ⇒ it IS emitted ⇒ removing it DOES change the compiled
    /// content_key (the earlier elision/omission WAS the `AnalysisEmissionDivergence`
    /// class). The `lqos-74t.29` dedup still drops it from the network, so its
    /// effect is now a REAL content change — pinned here.
    #[test]
    fn dedup_changes_the_compiled_content_key() {
        let with_node = serde_json::json!({
            "Site One": { "id": "uisp:site:one", "name": "Site One", "children": {
                "AP": { "id": "uisp:device:ap", "name": "AP", "children": {} } } },
            "LTU-2049BoyScout": { "id": "uisp:device:radio-1", "name": "LTU-2049BoyScout", "children": {} }
        });
        let mut without_node = with_node.clone();
        without_node
            .as_object_mut()
            .expect("object")
            .remove("LTU-2049BoyScout");
        let inputs = TopologyShapingInputsFile::default();
        let tx_with = crate::compile_dataplane_transaction(&with_node, &inputs);
        let tx_without = crate::compile_dataplane_transaction(&without_node, &inputs);
        assert_ne!(
            content_key(&tx_with),
            content_key(&tx_without),
            "(A1): a childless circuit-less top-level node IS emitted ⇒ removing it CHANGES the content"
        );
    }
}
