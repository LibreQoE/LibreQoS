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
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    effective_network: &Value,
    virtualization: &QueueVirtualizationContext,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let site_node_ids = canonical_site_node_ids(canonical);
    validate_effective_node_identity_consistency(ui_state, effective, &mut errors);
    validate_effective_site_parent_cycles(&site_node_ids, ui_state, effective, &mut errors);
    let queue_policy_tree = queue_policy_reference_tree(canonical, ui_state, effective)
        .unwrap_or_else(|_| Value::Object(Map::new()));
    let queue_policy_root = queue_policy_tree.as_object();

    let mut counts = HashMap::new();
    let Some(root) = effective_network.as_object() else {
        return Err(vec![
            "Effective topology export is not a JSON object tree.".to_string(),
        ]);
    };
    for child in root.values() {
        count_node_ids(child, &mut counts);
    }
    let child_branch_counts = logical_child_branch_counts(ui_state);
    let attachment_branch_counts = effective_attachment_branch_counts(effective);

    for canonical_node in canonical
        .nodes
        .iter()
        .filter(|node| site_node_ids.contains(node.node_id.as_str()))
    {
        let fallback_node;
        let ui_node = if let Some(ui_node) = ui_state.find_node(&canonical_node.node_id) {
            ui_node
        } else {
            fallback_node = TopologyEditorNode {
                node_id: canonical_node.node_id.clone(),
                node_name: canonical_node.node_name.clone(),
                queue_visibility_policy: canonical_node.queue_visibility_policy,
                ..TopologyEditorNode::default()
            };
            &fallback_node
        };
        if resolved_queue_visibility_policy(
            config,
            ui_node,
            queue_policy_root.and_then(|root| find_node_by_id(root, &canonical_node.node_id)),
            &child_branch_counts,
            &attachment_branch_counts,
            virtualization,
        ) == TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren
        {
            continue;
        }
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
        &QueueVirtualizationContext::default(),
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
    virtualization: &QueueVirtualizationContext,
) -> std::result::Result<Value, Vec<String>> {
    let mut out =
        apply_effective_topology_reparenting_only(canonical_network, ui_state, effective)?;
    if let Some(root) = out.as_object_mut() {
        recompile_effective_network_bandwidths(root, canonical, ui_state, effective);
        apply_queue_hidden_node_virtualization(config, ui_state, effective, root, virtualization);
        apply_runtime_squashing(config, ui_state, effective, root).map_err(|err| vec![err])?;
    }
    Ok(out)
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
        &QueueVirtualizationContext::default(),
    )
}

fn apply_effective_topology_to_canonical_state(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    virtualization: &QueueVirtualizationContext,
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
        virtualization,
    )
}
