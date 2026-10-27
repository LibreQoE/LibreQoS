fn node_type_is(value: &Value, expected: &str) -> bool {
    value
        .as_object()
        .and_then(|node| node.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|node_type| node_type == expected)
}

fn node_bandwidth_mbps(node: &Map<String, Value>, field: &str) -> Option<u64> {
    node.get(field).and_then(Value::as_u64).or_else(|| {
        node.get(field)
            .and_then(Value::as_f64)
            .map(|value| value as u64)
    })
}

fn min_chain_bandwidth(
    endpoint: &Map<String, Value>,
    relay_a: &Map<String, Value>,
    relay_b: &Map<String, Value>,
    field: &str,
) -> Option<u64> {
    [
        node_bandwidth_mbps(endpoint, field),
        node_bandwidth_mbps(relay_a, field),
        node_bandwidth_mbps(relay_b, field),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn min_attachment_bandwidth(
    endpoint: &Map<String, Value>,
    attachment: &Map<String, Value>,
    field: &str,
) -> Option<u64> {
    [
        node_bandwidth_mbps(endpoint, field),
        node_bandwidth_mbps(attachment, field),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn should_runtime_squash_chain(
    chain_names: [&str; 4],
    do_not_squash_sites: &HashSet<String>,
    direct_circuit_hosts: &DirectCircuitHosts,
) -> bool {
    // AP-anchor fix: never squash a chain that names a DIRECT-CIRCUIT node (an
    // anchored/hosting AP) — the config `do_not_squash_sites` OR the direct
    // NAME set. (Pass-2 checks child+grandchild; pass-3 the child — both ride
    // the `chain_names`.)
    //
    // NOTE: `chain_names` are the CHILD **MAP KEYS** (the node display NAMES),
    // so this gate is NAME-keyed. The ID-keyed guard for the same nodes lives at
    // each call site (`child_direct_id` / `grandchild_direct_id`), because the
    // node IDs are not the map keys.
    !chain_names.into_iter().any(|name| {
        do_not_squash_sites.contains(name)
            || direct_circuit_hosts.direct_circuit_node_names.contains(name)
    })
}

fn attachment_role_allows_runtime_squash(role: TopologyAttachmentRole) -> bool {
    matches!(
        role,
        TopologyAttachmentRole::PtpBackhaul | TopologyAttachmentRole::WiredUplink
    )
}

fn find_attachment_option_for_node<'a>(
    node: &'a TopologyEditorNode,
    parent_node_id: Option<&str>,
    attachment_id: Option<&str>,
) -> Option<&'a TopologyAttachmentOption> {
    let attachment_id = attachment_id?;
    node.allowed_parents
        .iter()
        .filter(|parent| {
            parent_node_id.is_none_or(|expected_parent| parent.parent_node_id == expected_parent)
        })
        .flat_map(|parent| parent.attachment_options.iter())
        .find(|option| option.attachment_id == attachment_id)
}

fn find_attachment_role_for_node(
    node: &TopologyEditorNode,
    parent_node_id: Option<&str>,
    attachment_id: Option<&str>,
) -> Option<TopologyAttachmentRole> {
    find_attachment_option_for_node(node, parent_node_id, attachment_id)
        .map(|option| option.attachment_role)
}

fn selected_attachment_roles(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> HashMap<String, TopologyAttachmentRole> {
    let mut roles_by_node_id = HashMap::new();
    let ui_by_node = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();

    for node in &ui_state.nodes {
        if let Some(role) = find_attachment_role_for_node(
            node,
            node.current_parent_node_id.as_deref(),
            node.current_attachment_id.as_deref(),
        ) {
            roles_by_node_id.insert(node.node_id.clone(), role);
        }
    }

    for effective_node in &effective.nodes {
        let Some(ui_node) = ui_by_node.get(effective_node.node_id.as_str()).copied() else {
            continue;
        };
        let Some(role) = find_attachment_role_for_node(
            ui_node,
            Some(effective_node.logical_parent_node_id.as_str()),
            effective_node.effective_attachment_id.as_deref(),
        ) else {
            continue;
        };
        roles_by_node_id.insert(effective_node.node_id.clone(), role);
    }

    roles_by_node_id
}

fn selected_attachment_pair_ids(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> HashSet<String> {
    let mut active_pair_ids = HashSet::new();
    let effective_node_ids = effective
        .nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<HashSet<_>>();
    let ui_by_node = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();

    for node in &ui_state.nodes {
        if effective_node_ids.contains(node.node_id.as_str()) {
            continue;
        }
        let Some(option) = find_attachment_option_for_node(
            node,
            node.current_parent_node_id.as_deref(),
            node.current_attachment_id.as_deref(),
        ) else {
            continue;
        };
        let Some(pair_id) = option.pair_id.as_ref() else {
            continue;
        };
        active_pair_ids.insert(pair_id.clone());
    }

    for effective_node in &effective.nodes {
        let Some(ui_node) = ui_by_node.get(effective_node.node_id.as_str()).copied() else {
            continue;
        };
        let Some(option) = find_attachment_option_for_node(
            ui_node,
            Some(effective_node.logical_parent_node_id.as_str()),
            effective_node.effective_attachment_id.as_deref(),
        ) else {
            continue;
        };
        let Some(pair_id) = option.pair_id.as_ref() else {
            continue;
        };
        active_pair_ids.insert(pair_id.clone());
    }

    active_pair_ids
}

fn attachment_pair_memberships(
    ui_state: &TopologyEditorStateFile,
) -> HashMap<String, (TopologyAttachmentRole, String)> {
    let mut pair_by_attachment_id = HashMap::new();
    for node in &ui_state.nodes {
        for parent in &node.allowed_parents {
            for option in &parent.attachment_options {
                let Some(pair_id) = option.pair_id.as_ref() else {
                    continue;
                };
                if !attachment_role_allows_runtime_squash(option.attachment_role) {
                    continue;
                }
                pair_by_attachment_id.insert(
                    option.attachment_id.clone(),
                    (option.attachment_role, pair_id.clone()),
                );
                if let Some(peer_attachment_id) = option.peer_attachment_id.as_ref() {
                    pair_by_attachment_id.insert(
                        peer_attachment_id.clone(),
                        (option.attachment_role, pair_id.clone()),
                    );
                }
            }
        }
    }
    pair_by_attachment_id
}

fn endpoint_attachment_role(
    endpoint_node: &Map<String, Value>,
    roles_by_node_id: &HashMap<String, TopologyAttachmentRole>,
) -> TopologyAttachmentRole {
    endpoint_node
        .get("id")
        .and_then(Value::as_str)
        .and_then(|node_id| roles_by_node_id.get(node_id).copied())
        .unwrap_or_default()
}

fn is_inactive_backhaul_stub_subtree(
    node: &Map<String, Value>,
    pair_by_attachment_id: &HashMap<String, (TopologyAttachmentRole, String)>,
    active_pair_ids: &HashSet<String>,
    direct_circuit_hosts: &DirectCircuitHosts,
) -> bool {
    let Some(node_id) = node.get("id").and_then(Value::as_str) else {
        return false;
    };
    // AP-anchor fix: an inactive backhaul stub that ANCHORS/HOSTS a direct
    // circuit must NOT be pruned. The guard checks BOTH the id AND the name set,
    // because node IDs are not the map keys.
    let node_name = node.get("name").and_then(Value::as_str).unwrap_or("");
    if direct_circuit_hosts.direct_circuit_node_ids.contains(node_id)
        || direct_circuit_hosts.direct_circuit_node_names.contains(node_name)
    {
        return false;
    }
    let Some((role, pair_id)) = pair_by_attachment_id.get(node_id) else {
        return false;
    };
    if !attachment_role_allows_runtime_squash(*role) || active_pair_ids.contains(pair_id) {
        return false;
    }
    let Some(children) = node.get("children").and_then(Value::as_object) else {
        return true;
    };
    children.values().all(|child| {
        let Some(child_node) = child.as_object() else {
            return false;
        };
        node_type_is(child, "AP")
            && is_inactive_backhaul_stub_subtree(
                child_node,
                pair_by_attachment_id,
                active_pair_ids,
                direct_circuit_hosts,
            )
    })
}

fn prune_inactive_backhaul_stubs_in_children(
    children: &mut Map<String, Value>,
    pair_by_attachment_id: &HashMap<String, (TopologyAttachmentRole, String)>,
    active_pair_ids: &HashSet<String>,
    direct_circuit_hosts: &DirectCircuitHosts,
) {
    let child_keys = children.keys().cloned().collect::<Vec<_>>();
    for child_key in child_keys {
        let Some(node) = children.get_mut(&child_key).and_then(Value::as_object_mut) else {
            continue;
        };
        let Some(grandchildren) = node.get_mut("children").and_then(Value::as_object_mut) else {
            continue;
        };
        prune_inactive_backhaul_stubs_in_children(
            grandchildren,
            pair_by_attachment_id,
            active_pair_ids,
            direct_circuit_hosts,
        );
    }

    let child_keys = children.keys().cloned().collect::<Vec<_>>();
    for child_key in child_keys {
        let should_remove = children
            .get(&child_key)
            .and_then(Value::as_object)
            .is_some_and(|node| {
                is_inactive_backhaul_stub_subtree(
                    node,
                    pair_by_attachment_id,
                    active_pair_ids,
                    direct_circuit_hosts,
                )
            });
        if should_remove {
            children.remove(&child_key);
        }
    }
}

fn squash_backhaul_pairs_in_children(
    parent_name: Option<&str>,
    children: &mut Map<String, Value>,
    do_not_squash_sites: &HashSet<String>,
    roles_by_node_id: &HashMap<String, TopologyAttachmentRole>,
    direct_circuit_hosts: &DirectCircuitHosts,
) -> std::result::Result<(), String> {
    let child_keys = children.keys().cloned().collect::<Vec<_>>();
    for child_key in child_keys {
        let Some(node) = children.get_mut(&child_key).and_then(Value::as_object_mut) else {
            continue;
        };
        let Some(grandchildren) = node.get_mut("children").and_then(Value::as_object_mut) else {
            continue;
        };
        squash_backhaul_pairs_in_children(
            Some(&child_key),
            grandchildren,
            do_not_squash_sites,
            roles_by_node_id,
            direct_circuit_hosts,
        )?;
    }

    loop {
        let mut changed = false;
        let child_keys = children.keys().cloned().collect::<Vec<_>>();
        for child_key in child_keys {
            let Some(child_value) = children.get(&child_key) else {
                continue;
            };
            if !node_type_is(child_value, "AP") {
                continue;
            }
            let Some(child_node) = child_value.as_object() else {
                continue;
            };
            let Some(child_children) = child_node.get("children").and_then(Value::as_object) else {
                continue;
            };
            if child_children.len() != 1 {
                continue;
            }
            let Some((grandchild_key, grandchild_value)) = child_children.iter().next() else {
                continue;
            };
            let grandchild_key = grandchild_key.clone();
            if !node_type_is(grandchild_value, "AP") {
                continue;
            }
            let Some(grandchild_node) = grandchild_value.as_object() else {
                continue;
            };
            let Some(grandchild_children) =
                grandchild_node.get("children").and_then(Value::as_object)
            else {
                continue;
            };
            if grandchild_children.len() != 1 {
                continue;
            }
            let Some((endpoint_key, endpoint_value)) = grandchild_children.iter().next() else {
                continue;
            };
            let endpoint_key = endpoint_key.clone();
            if node_type_is(endpoint_value, "AP") {
                continue;
            }
            let Some(endpoint_node) = endpoint_value.as_object() else {
                continue;
            };
            if !attachment_role_allows_runtime_squash(endpoint_attachment_role(
                endpoint_node,
                roles_by_node_id,
            )) {
                continue;
            }
            // AP-anchor fix: never squash a chain whose CHILD or GRANDCHILD
            // hosts/anchors a direct circuit (id OR name).
            let child_direct_id = child_node
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| direct_circuit_hosts.direct_circuit_node_ids.contains(id));
            let grandchild_direct_id = grandchild_node
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| direct_circuit_hosts.direct_circuit_node_ids.contains(id));
            if child_direct_id || grandchild_direct_id {
                continue;
            }
            if !should_runtime_squash_chain(
                [
                    parent_name.unwrap_or_default(),
                    &child_key,
                    &grandchild_key,
                    &endpoint_key,
                ],
                do_not_squash_sites,
                direct_circuit_hosts,
            ) {
                continue;
            }
            if endpoint_key != child_key && children.contains_key(&endpoint_key) {
                return Err(format!(
                    "Unable to squash runtime backhaul pair '{child_key}' into '{endpoint_key}' under '{}': child key already exists",
                    parent_name.unwrap_or("<root>")
                ));
            }

            let Some(mut child_value) = children.remove(&child_key) else {
                continue;
            };
            let Some(child_node) = child_value.as_object_mut() else {
                continue;
            };
            let Some(child_children) = child_node
                .get_mut("children")
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            let Some(mut grandchild_value) = child_children.remove(&grandchild_key) else {
                continue;
            };
            let Some(grandchild_node) = grandchild_value.as_object_mut() else {
                continue;
            };
            let Some(grandchild_children) = grandchild_node
                .get_mut("children")
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            let Some(mut endpoint_value) = grandchild_children.remove(&endpoint_key) else {
                continue;
            };
            let Some(endpoint_node) = endpoint_value.as_object_mut() else {
                continue;
            };

            if let Some(download) = min_chain_bandwidth(
                endpoint_node,
                child_node,
                grandchild_node,
                "downloadBandwidthMbps",
            ) {
                endpoint_node.insert(
                    "downloadBandwidthMbps".to_string(),
                    Value::Number(download.into()),
                );
            }
            if let Some(upload) = min_chain_bandwidth(
                endpoint_node,
                child_node,
                grandchild_node,
                "uploadBandwidthMbps",
            ) {
                endpoint_node.insert(
                    "uploadBandwidthMbps".to_string(),
                    Value::Number(upload.into()),
                );
            }
            if let Some(parent_name) = parent_name {
                endpoint_node.insert(
                    "parent_site".to_string(),
                    Value::String(parent_name.to_string()),
                );
            }
            endpoint_node
                .entry("name".to_string())
                .or_insert_with(|| Value::String(endpoint_key.clone()));
            endpoint_node.insert(
                "active_attachment_name".to_string(),
                Value::String(grandchild_key.clone()),
            );

            children.insert(endpoint_key.clone(), endpoint_value);
            changed = true;
            break;
        }

        if !changed {
            break;
        }
    }
    Ok(())
}

fn squash_single_attachment_hops_in_children(
    parent_name: Option<&str>,
    children: &mut Map<String, Value>,
    do_not_squash_sites: &HashSet<String>,
    roles_by_node_id: &HashMap<String, TopologyAttachmentRole>,
    direct_circuit_hosts: &DirectCircuitHosts,
) -> std::result::Result<(), String> {
    let child_keys = children.keys().cloned().collect::<Vec<_>>();
    for child_key in child_keys {
        let Some(node) = children.get_mut(&child_key).and_then(Value::as_object_mut) else {
            continue;
        };
        let Some(grandchildren) = node.get_mut("children").and_then(Value::as_object_mut) else {
            continue;
        };
        squash_single_attachment_hops_in_children(
            Some(&child_key),
            grandchildren,
            do_not_squash_sites,
            roles_by_node_id,
            direct_circuit_hosts,
        )?;
    }

    loop {
        let mut changed = false;
        let child_keys = children.keys().cloned().collect::<Vec<_>>();
        for child_key in child_keys {
            let Some(child_value) = children.get(&child_key) else {
                continue;
            };
            if !node_type_is(child_value, "AP") {
                continue;
            }
            let Some(child_node) = child_value.as_object() else {
                continue;
            };
            let Some(child_children) = child_node.get("children").and_then(Value::as_object) else {
                continue;
            };
            if child_children.len() != 1 {
                continue;
            }
            let Some((endpoint_key, endpoint_value)) = child_children.iter().next() else {
                continue;
            };
            let endpoint_key = endpoint_key.clone();
            if node_type_is(endpoint_value, "AP") {
                continue;
            }
            let Some(endpoint_node) = endpoint_value.as_object() else {
                continue;
            };
            if !attachment_role_allows_runtime_squash(endpoint_attachment_role(
                endpoint_node,
                roles_by_node_id,
            )) {
                continue;
            }
            // AP-anchor fix: never squash a chain whose CHILD hosts/anchors a
            // direct circuit (id OR name).
            let child_direct_id = child_node
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| direct_circuit_hosts.direct_circuit_node_ids.contains(id));
            if child_direct_id {
                continue;
            }
            if !should_runtime_squash_chain(
                [
                    parent_name.unwrap_or_default(),
                    &child_key,
                    &endpoint_key,
                    "",
                ],
                do_not_squash_sites,
                direct_circuit_hosts,
            ) {
                continue;
            }
            if endpoint_key != child_key && children.contains_key(&endpoint_key) {
                return Err(format!(
                    "Unable to squash runtime attachment hop '{child_key}' into '{endpoint_key}' under '{}': child key already exists",
                    parent_name.unwrap_or("<root>")
                ));
            }

            let Some(mut child_value) = children.remove(&child_key) else {
                continue;
            };
            let Some(child_node) = child_value.as_object_mut() else {
                continue;
            };
            let Some(child_children) = child_node
                .get_mut("children")
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            let Some(mut endpoint_value) = child_children.remove(&endpoint_key) else {
                continue;
            };
            let Some(endpoint_node) = endpoint_value.as_object_mut() else {
                continue;
            };

            if let Some(download) =
                min_attachment_bandwidth(endpoint_node, child_node, "downloadBandwidthMbps")
            {
                endpoint_node.insert(
                    "downloadBandwidthMbps".to_string(),
                    Value::Number(download.into()),
                );
            }
            if let Some(upload) =
                min_attachment_bandwidth(endpoint_node, child_node, "uploadBandwidthMbps")
            {
                endpoint_node.insert(
                    "uploadBandwidthMbps".to_string(),
                    Value::Number(upload.into()),
                );
            }
            if let Some(parent_name) = parent_name {
                endpoint_node.insert(
                    "parent_site".to_string(),
                    Value::String(parent_name.to_string()),
                );
            }
            endpoint_node
                .entry("name".to_string())
                .or_insert_with(|| Value::String(endpoint_key.clone()));
            endpoint_node.insert(
                "active_attachment_name".to_string(),
                Value::String(child_key.clone()),
            );

            children.insert(endpoint_key.clone(), endpoint_value);
            changed = true;
            break;
        }

        if !changed {
            break;
        }
    }
    Ok(())
}

fn apply_runtime_squashing(
    config: &Config,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    direct_circuit_hosts: &DirectCircuitHosts,
    root: &mut Map<String, Value>,
) -> std::result::Result<(), String> {
    if !ui_state.source.starts_with("uisp/") {
        return Ok(());
    }
    if !config.uisp_integration.enable_uisp {
        return Ok(());
    }

    let do_not_squash_sites = config
        .uisp_integration
        .do_not_squash_sites
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();
    let active_pair_ids = selected_attachment_pair_ids(ui_state, effective);
    let pair_by_attachment_id = attachment_pair_memberships(ui_state);
    prune_inactive_backhaul_stubs_in_children(
        root,
        &pair_by_attachment_id,
        &active_pair_ids,
        direct_circuit_hosts,
    );
    let roles_by_node_id = selected_attachment_roles(ui_state, effective);
    squash_backhaul_pairs_in_children(
        None,
        root,
        &do_not_squash_sites,
        &roles_by_node_id,
        direct_circuit_hosts,
    )?;
    squash_single_attachment_hops_in_children(
        None,
        root,
        &do_not_squash_sites,
        &roles_by_node_id,
        direct_circuit_hosts,
    )?;
    Ok(())
}

#[cfg(test)]
mod ap_anchor_tests {
    use super::*;

    fn ctx(ids: &[&str], names: &[&str]) -> DirectCircuitHosts {
        DirectCircuitHosts {
            direct_circuit_node_ids: ids.iter().map(|s| s.to_string()).collect(),
            direct_circuit_node_names: names.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn inactive_stub_pair() -> (
        HashMap<String, (TopologyAttachmentRole, String)>,
        HashSet<String>,
    ) {
        let pair_by = HashMap::from([(
            "ap1".to_string(),
            (TopologyAttachmentRole::PtpBackhaul, "pair-x".to_string()),
        )]);
        // pair-x is INACTIVE (not in active_pair_ids).
        (pair_by, HashSet::new())
    }

    fn stub_node() -> Map<String, Value> {
        serde_json::json!({
            "id": "ap1",
            "name": "JR_AP_TR_D",
            "type": "AP",
            "children": {},
        })
        .as_object()
        .expect("object")
        .clone()
    }

    /// AP-anchor fix oracle: an inactive backhaul stub AP that HOSTS/ANCHORS a
    /// direct circuit is EXEMPT (id OR name); a stub with no direct circuit still
    /// prunes.
    #[test]
    fn anchored_backhaul_stub_is_not_pruned_by_id_or_name() {
        let (pair_by, active) = inactive_stub_pair();
        let node = stub_node();
        assert!(
            is_inactive_backhaul_stub_subtree(&node, &pair_by, &active, &ctx(&[], &[])),
            "an inactive stub with no direct circuit must still prune"
        );
        assert!(
            !is_inactive_backhaul_stub_subtree(&node, &pair_by, &active, &ctx(&["ap1"], &[])),
            "an in_direct-id stub must be exempt"
        );
        assert!(
            !is_inactive_backhaul_stub_subtree(
                &node,
                &pair_by,
                &active,
                &ctx(&[], &["JR_AP_TR_D"])
            ),
            "a name-only direct stub must be exempt (the FULL exemption mirrors both sets)"
        );
    }

    /// AP-anchor fix oracle: `should_runtime_squash_chain` (pass-2 child+grandchild
    /// and pass-3 child ride `chain_names`) blocks a chain that NAMES a direct
    /// circuit node.
    #[test]
    fn should_squash_chain_blocks_a_direct_named_node() {
        let empty = HashSet::<String>::new();
        assert!(
            should_runtime_squash_chain(
                ["parent", "child", "grandchild", "endpoint"],
                &empty,
                &ctx(&[], &["child"])
            ) == false,
            "a direct CHILD name must block the squash"
        );
        assert!(
            should_runtime_squash_chain(
                ["parent", "child", "grandchild", "endpoint"],
                &empty,
                &ctx(&[], &["grandchild"])
            ) == false,
            "a direct GRANDCHILD name must block the squash"
        );
        assert!(
            should_runtime_squash_chain(
                ["parent", "child", "grandchild", "endpoint"],
                &empty,
                &ctx(&[], &[])
            ),
            "no direct name -> the squash is allowed"
        );
    }
    fn children_of(value: Value) -> Map<String, Value> {
        value.as_object().expect("object").clone()
    }

    fn squash_roles() -> HashMap<String, TopologyAttachmentRole> {
        HashMap::from([("ep1".to_string(), TopologyAttachmentRole::PtpBackhaul)])
    }

    fn empty_sites() -> HashSet<String> {
        HashSet::new()
    }

    fn pass3_tree() -> Map<String, Value> {
        children_of(serde_json::json!({
            "child": {"id": "ap1", "name": "child", "type": "AP", "children": {
                "ep": {"id": "ep1", "name": "ep", "type": "Switch", "children": {}}
            }}
        }))
    }

    fn pass2_tree() -> Map<String, Value> {
        children_of(serde_json::json!({
            "child": {"id": "ap1", "name": "child", "type": "AP", "children": {
                "gc": {"id": "ap2", "name": "gc", "type": "AP", "children": {
                    "ep": {"id": "ep1", "name": "ep", "type": "Switch", "children": {}}
                }}
            }}
        }))
    }

    /// (a)+(b) pass-3 end-to-end: the EXPLICIT child-id guard prevents the
    /// single-attachment-hop squash of an anchored child; with no direct child the
    /// hop IS squashed (the optimization).
    #[test]
    fn pass3_hop_survives_when_the_child_is_direct_by_id() {
        let roles = squash_roles();
        let empty = empty_sites();
        let mut children = pass3_tree();
        squash_single_attachment_hops_in_children(
            None,
            &mut children,
            &empty,
            &roles,
            &ctx(&["ap1"], &[]),
        )
        .expect("squash");
        assert!(
            children.contains_key("child"),
            "pass-3 must NOT remove a direct (anchored) child"
        );
        let mut children = pass3_tree();
        squash_single_attachment_hops_in_children(None, &mut children, &empty, &roles, &ctx(&[], &[]))
            .expect("squash");
        assert!(!children.contains_key("child"), "pass-3 squashes when nothing is direct");
        assert!(children.contains_key("ep"), "the endpoint is promoted");
    }

    /// (a)+(b) pass-2 end-to-end: the EXPLICIT child-or-grandchild id guard
    /// prevents the backhaul-pair squash; with no direct node it IS squashed.
    #[test]
    fn pass2_pair_survives_when_child_or_grandchild_is_direct_by_id() {
        let roles = squash_roles();
        let empty = empty_sites();
        for direct in [vec!["ap1"], vec!["ap2"]] {
            let mut children = pass2_tree();
            squash_backhaul_pairs_in_children(
                None,
                &mut children,
                &empty,
                &roles,
                &ctx(&direct, &[]),
            )
            .expect("squash");
            assert!(
                children.contains_key("child"),
                "pass-2 must NOT remove a child/grandchild direct by id: {direct:?}"
            );
        }
        let mut children = pass2_tree();
        squash_backhaul_pairs_in_children(None, &mut children, &empty, &roles, &ctx(&[], &[]))
            .expect("squash");
        assert!(!children.contains_key("child"), "pass-2 squashes when nothing is direct");
        assert!(children.contains_key("ep"), "the endpoint is promoted");
    }
}

