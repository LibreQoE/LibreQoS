fn remove_node_by_id(map: &mut Map<String, Value>, target_id: &str) -> Option<(String, Value)> {
    let keys = map.keys().cloned().collect::<Vec<_>>();
    for key in keys {
        let Some(value) = map.get_mut(&key) else {
            continue;
        };
        let Some(node) = value.as_object_mut() else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == target_id)
        {
            let removed = map.remove(&key)?;
            return Some((key, removed));
        }
        if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut)
            && let Some(found) = remove_node_by_id(children, target_id)
        {
            return Some(found);
        }
    }
    None
}

fn logical_child_branch_counts(ui_state: &TopologyEditorStateFile) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for node in &ui_state.nodes {
        let Some(parent_id) = node.current_parent_node_id.as_deref() else {
            continue;
        };
        *counts.entry(parent_id.to_string()).or_insert(0) += 1;
    }
    counts
}

fn effective_attachment_branch_counts(
    effective: &TopologyEffectiveStateFile,
) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for node in &effective.nodes {
        let Some(attachment_id) = node.effective_attachment_id.as_deref() else {
            continue;
        };
        if attachment_id.trim().is_empty() {
            continue;
        }
        *counts.entry(attachment_id.to_string()).or_insert(0) += 1;
    }
    counts
}

fn read_node_rate_mbps(node: &Map<String, Value>, key: &str) -> Option<u64> {
    node.get(key).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_f64().map(|rate| rate as u64))
    })
}

fn node_capacity_mbps(node: &Map<String, Value>) -> u64 {
    let download = read_node_rate_mbps(node, "downloadBandwidthMbps").unwrap_or_default();
    let upload = read_node_rate_mbps(node, "uploadBandwidthMbps").unwrap_or_default();
    download.max(upload)
}

fn resolved_queue_visibility_policy(
    config: &Config,
    ui_node: &TopologyEditorNode,
    tree_node: Option<&Value>,
    child_branch_counts: &HashMap<String, usize>,
    attachment_branch_counts: &HashMap<String, usize>,
    virtualization: &QueueVirtualizationContext,
) -> TopologyQueueVisibilityPolicy {
    if virtualization
        .forced_visible_node_names
        .contains(ui_node.node_name.as_str())
    {
        return TopologyQueueVisibilityPolicy::QueueVisible;
    }
    if virtualization
        .direct_circuit_node_ids
        .contains(ui_node.node_id.as_str())
        || virtualization
            .direct_circuit_node_names
            .contains(ui_node.node_name.as_str())
    {
        return TopologyQueueVisibilityPolicy::QueueVisible;
    }
    match ui_node.queue_visibility_policy {
        TopologyQueueVisibilityPolicy::QueueVisible => TopologyQueueVisibilityPolicy::QueueVisible,
        TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren => {
            TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren
        }
        TopologyQueueVisibilityPolicy::QueueAuto => resolved_auto_queue_visibility_policy(
            config,
            ui_node,
            tree_node,
            child_branch_counts,
            attachment_branch_counts,
            virtualization,
        ),
    }
}

fn resolved_auto_queue_visibility_policy(
    config: &Config,
    ui_node: &TopologyEditorNode,
    tree_node: Option<&Value>,
    child_branch_counts: &HashMap<String, usize>,
    attachment_branch_counts: &HashMap<String, usize>,
    _virtualization: &QueueVirtualizationContext,
) -> TopologyQueueVisibilityPolicy {
    let Some(tree_node) = tree_node.and_then(Value::as_object) else {
        return TopologyQueueVisibilityPolicy::QueueVisible;
    };
    if !queue_auto_node_kind_can_hide(tree_node) {
        return TopologyQueueVisibilityPolicy::QueueVisible;
    }
    let logical_child_count = child_branch_counts
        .get(ui_node.node_id.as_str())
        .copied()
        .unwrap_or_default();
    let attachment_child_count = attachment_branch_counts
        .get(ui_node.node_id.as_str())
        .copied()
        .unwrap_or_default();
    if logical_child_count == 0 && attachment_child_count == 0 {
        return TopologyQueueVisibilityPolicy::QueueVisible;
    }
    let threshold = config.topology.queue_auto_virtualize_threshold_mbps;
    if threshold == 0 {
        return TopologyQueueVisibilityPolicy::QueueVisible;
    }
    if node_capacity_mbps(tree_node) >= threshold {
        TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren
    } else {
        TopologyQueueVisibilityPolicy::QueueVisible
    }
}

fn queue_auto_node_kind_can_hide(tree_node: &Map<String, Value>) -> bool {
    tree_node
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind.to_ascii_lowercase().as_str(), "site" | "ap"))
}

fn apply_effective_topology_reparenting_only(
    canonical_network: &Value,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> std::result::Result<Value, Vec<String>> {
    let Some(root) = canonical_network.as_object() else {
        return Ok(canonical_network.clone());
    };
    let mut out = root.clone();
    let ui_by_node = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();

    for effective_node in &effective.nodes {
        let Some(ui_node) = ui_by_node.get(effective_node.node_id.as_str()).copied() else {
            continue;
        };
        let Some(selected_parent) = ui_node
            .allowed_parents
            .iter()
            .find(|parent| parent.parent_node_id == effective_node.logical_parent_node_id)
        else {
            continue;
        };
        let already_parented = find_parent_id_of_node(&out, &ui_node.node_id, None)
            .flatten()
            .as_deref()
            == Some(selected_parent.parent_node_id.as_str());
        let Some(effective_attachment_id) = effective_node.effective_attachment_id.as_deref()
        else {
            if ui_node.current_parent_node_id.as_deref()
                == Some(effective_node.logical_parent_node_id.as_str())
                && already_parented
            {
                continue;
            }
            let Some((node_key, node_value)) = remove_node_by_id(&mut out, &ui_node.node_id) else {
                continue;
            };
            insert_node_under_parent_id(
                &mut out,
                &selected_parent.parent_node_id,
                &node_key,
                node_value,
            )
            .map_err(|err| vec![err])?;
            continue;
        };
        let Some(target_attachment) = selected_parent
            .attachment_options
            .iter()
            .find(|option| option.attachment_id == effective_attachment_id)
        else {
            continue;
        };
        let current_anchor_attachment = find_node_by_id(&out, &ui_node.node_id)
            .map(|node_value| attachment_anchor_for_reparent(node_value, target_attachment))
            .unwrap_or_else(|| target_attachment.clone());
        let already_anchored = already_parented
            && current_anchor_attachment.attachment_id == selected_parent.parent_node_id
            || find_parent_id_of_node(&out, &ui_node.node_id, None)
                .flatten()
                .as_deref()
                == Some(current_anchor_attachment.attachment_id.as_str());
        if ui_node.current_parent_node_id.as_deref()
            == Some(effective_node.logical_parent_node_id.as_str())
            && ui_node.current_attachment_id.as_deref()
                == effective_node.effective_attachment_id.as_deref()
            && already_anchored
        {
            if should_anchor_reparent_under_attachment(&ui_node.node_id, &current_anchor_attachment)
            {
                ensure_attachment_node_exists(
                    &mut out,
                    &selected_parent.parent_node_id,
                    &current_anchor_attachment,
                )
                .map_err(|err| vec![err])?;
            }
            continue;
        }

        let Some((node_key, node_value)) = remove_node_by_id(&mut out, &ui_node.node_id) else {
            continue;
        };
        let anchor_attachment = attachment_anchor_for_reparent(&node_value, target_attachment);
        if should_anchor_reparent_under_attachment(&ui_node.node_id, &anchor_attachment) {
            ensure_attachment_node_exists(
                &mut out,
                &selected_parent.parent_node_id,
                &anchor_attachment,
            )
            .map_err(|err| vec![err])?;
            insert_node_under_parent_id(
                &mut out,
                &anchor_attachment.attachment_id,
                &node_key,
                node_value,
            )
            .map_err(|err| vec![err])?;
        } else {
            insert_node_under_parent_id(
                &mut out,
                &selected_parent.parent_node_id,
                &node_key,
                node_value,
            )
            .map_err(|err| vec![err])?;
        }
    }

    Ok(Value::Object(out))
}

fn queue_policy_reference_tree(
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> std::result::Result<Value, Vec<String>> {
    let canonical_network =
        if canonical.ingress_kind == TopologyCanonicalIngressKind::NativeIntegration {
            canonical.insight_topology_network_json()
        } else {
            canonical.compatibility_network_json().clone()
        };
    let mut logical_tree =
        apply_effective_topology_reparenting_only(&canonical_network, ui_state, effective)?;
    if let Some(root) = logical_tree.as_object_mut() {
        recompile_effective_network_bandwidths(root, canonical, ui_state, effective);
    }
    Ok(logical_tree)
}

fn queue_hidden_node_ids_in_promotion_order(ui_state: &TopologyEditorStateFile) -> Vec<String> {
    let by_id = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut depth_cache = HashMap::<String, usize>::new();

    fn node_depth<'a>(
        node_id: &'a str,
        by_id: &HashMap<&'a str, &'a TopologyEditorNode>,
        cache: &mut HashMap<String, usize>,
        seen: &mut HashSet<String>,
    ) -> usize {
        if let Some(depth) = cache.get(node_id).copied() {
            return depth;
        }
        if !seen.insert(node_id.to_string()) {
            return 0;
        }
        let depth = by_id
            .get(node_id)
            .and_then(|node| node.current_parent_node_id.as_deref())
            .map(|parent_id| 1 + node_depth(parent_id, by_id, cache, seen))
            .unwrap_or(0);
        seen.remove(node_id);
        cache.insert(node_id.to_string(), depth);
        depth
    }

    let mut node_ids = ui_state
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect::<Vec<_>>();
    node_ids.sort_unstable_by(|left, right| {
        let left_depth = node_depth(left, &by_id, &mut depth_cache, &mut HashSet::new());
        let right_depth = node_depth(right, &by_id, &mut depth_cache, &mut HashSet::new());
        left_depth.cmp(&right_depth).then_with(|| left.cmp(right))
    });
    node_ids
}

fn mark_node_virtual_by_id(map: &mut Map<String, Value>, target_id: &str) -> bool {
    for value in map.values_mut() {
        let Some(node) = value.as_object_mut() else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == target_id)
        {
            node.insert("virtual".to_string(), Value::Bool(true));
            return true;
        }
        if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut)
            && mark_node_virtual_by_id(children, target_id)
        {
            return true;
        }
    }
    false
}

fn apply_queue_hidden_node_virtualization(
    config: &Config,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    root: &mut Map<String, Value>,
    virtualization: &QueueVirtualizationContext,
) {
    let ui_by_id = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let child_branch_counts = logical_child_branch_counts(ui_state);
    let attachment_branch_counts = effective_attachment_branch_counts(effective);
    let hidden_node_ids = queue_hidden_node_ids_in_promotion_order(ui_state);
    for hidden_node_id in hidden_node_ids {
        let Some(ui_node) = ui_by_id.get(hidden_node_id.as_str()).copied() else {
            continue;
        };
        let resolved_policy = resolved_queue_visibility_policy(
            config,
            ui_node,
            find_node_by_id(root, &hidden_node_id),
            &child_branch_counts,
            &attachment_branch_counts,
            virtualization,
        );
        if resolved_policy != TopologyQueueVisibilityPolicy::QueueHiddenPromoteChildren {
            continue;
        }
        let _ = mark_node_virtual_by_id(root, &hidden_node_id);
    }
}

fn find_node_by_id<'a>(map: &'a Map<String, Value>, target_id: &str) -> Option<&'a Value> {
    for value in map.values() {
        let Some(node) = value.as_object() else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == target_id)
        {
            return Some(value);
        }
        if let Some(children) = node.get("children").and_then(Value::as_object)
            && let Some(found) = find_node_by_id(children, target_id)
        {
            return Some(found);
        }
    }
    None
}

fn find_parent_id_of_node(
    map: &Map<String, Value>,
    target_id: &str,
    current_parent_id: Option<&str>,
) -> Option<Option<String>> {
    for value in map.values() {
        let Some(node) = value.as_object() else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == target_id)
        {
            return Some(current_parent_id.map(ToOwned::to_owned));
        }
        if let Some(children) = node.get("children").and_then(Value::as_object)
            && let Some(found) =
                find_parent_id_of_node(children, target_id, node.get("id").and_then(Value::as_str))
        {
            return Some(found);
        }
    }
    None
}

fn value_subtree_contains_id(value: &Value, target_id: &str) -> bool {
    let Some(node) = value.as_object() else {
        return false;
    };
    if node
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| id == target_id)
    {
        return true;
    }
    node.get("children")
        .and_then(Value::as_object)
        .is_some_and(|children| {
            children
                .values()
                .any(|child| value_subtree_contains_id(child, target_id))
        })
}

fn insert_node_under_parent_id(
    map: &mut Map<String, Value>,
    parent_id: &str,
    node_key: &str,
    node_value: Value,
) -> std::result::Result<(), String> {
    fn insert_node_under_parent_id_inner(
        map: &mut Map<String, Value>,
        parent_id: &str,
        node_key: &str,
        node_value: &Value,
    ) -> std::result::Result<bool, String> {
        for (key, value) in map.iter_mut() {
            let Some(node) = value.as_object_mut() else {
                continue;
            };
            if key == parent_id
                || node
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id == parent_id)
            {
                let parent_name = node
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(key)
                    .to_string();
                let children = node
                    .entry("children".to_string())
                    .or_insert_with(|| Value::Object(Map::new()));
                let Some(children) = children.as_object_mut() else {
                    return Err(format!(
                        "Unable to reparent '{node_key}' under '{parent_id}': parent has non-object children"
                    ));
                };
                if children.contains_key(node_key) {
                    return Err(format!(
                        "Unable to reparent '{node_key}' under '{parent_id}': child key already exists"
                    ));
                }
                let mut node_value = node_value.clone();
                if let Some(node_object) = node_value.as_object_mut() {
                    node_object.insert("parent_site".to_string(), Value::String(parent_name));
                    node_object
                        .entry("name".to_string())
                        .or_insert_with(|| Value::String(node_key.to_string()));
                }
                children.insert(node_key.to_string(), node_value);
                return Ok(true);
            }
            if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut)
                && insert_node_under_parent_id_inner(children, parent_id, node_key, node_value)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    if insert_node_under_parent_id_inner(map, parent_id, node_key, &node_value)? {
        Ok(())
    } else {
        Err(format!(
            "Unable to reparent '{node_key}': target parent '{parent_id}' was not found"
        ))
    }
}

fn ensure_attachment_node_exists(
    root: &mut Map<String, Value>,
    parent_id: &str,
    attachment: &TopologyAttachmentOption,
) -> std::result::Result<(), String> {
    if update_node_bandwidths_by_id(root, &attachment.attachment_id, attachment) {
        return Ok(());
    }
    let download = attachment
        .download_bandwidth_mbps
        .or(attachment.capacity_mbps)
        .unwrap_or(0);
    let upload = attachment
        .upload_bandwidth_mbps
        .or(attachment.capacity_mbps)
        .unwrap_or(0);
    let mut node = Map::new();
    node.insert("children".to_string(), Value::Object(Map::new()));
    node.insert(
        "downloadBandwidthMbps".to_string(),
        Value::Number(download.into()),
    );
    node.insert(
        "uploadBandwidthMbps".to_string(),
        Value::Number(upload.into()),
    );
    node.insert(
        "id".to_string(),
        Value::String(attachment.attachment_id.clone()),
    );
    node.insert(
        "name".to_string(),
        Value::String(attachment.attachment_name.clone()),
    );
    node.insert("type".to_string(), Value::String("AP".to_string()));
    insert_node_under_parent_id(
        root,
        parent_id,
        &attachment.attachment_name,
        Value::Object(node),
    )
    .map_err(|err| {
        format!(
            "Unable to create attachment node '{}' under '{}': {err}",
            attachment.attachment_name, parent_id
        )
    })
}

fn attachment_anchor_for_reparent(
    moved_subtree: &Value,
    attachment: &TopologyAttachmentOption,
) -> TopologyAttachmentOption {
    let Some(peer_attachment_id) = attachment.peer_attachment_id.as_ref() else {
        return attachment.clone();
    };
    if !value_subtree_contains_id(moved_subtree, &attachment.attachment_id) {
        return attachment.clone();
    }

    let mut anchor = attachment.clone();
    anchor.attachment_id = peer_attachment_id.clone();
    anchor.attachment_name = attachment
        .peer_attachment_name
        .clone()
        .unwrap_or_else(|| peer_attachment_id.clone());
    anchor.peer_attachment_id = Some(attachment.attachment_id.clone());
    anchor.peer_attachment_name = Some(attachment.attachment_name.clone());
    anchor
}

fn should_anchor_reparent_under_attachment(
    node_id: &str,
    attachment: &TopologyAttachmentOption,
) -> bool {
    attachment.attachment_id != node_id
}

fn update_node_bandwidths_by_id(
    root: &mut Map<String, Value>,
    node_id: &str,
    attachment: &TopologyAttachmentOption,
) -> bool {
    for value in root.values_mut() {
        let Some(node) = value.as_object_mut() else {
            continue;
        };
        if node
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == node_id)
        {
            let download = attachment
                .download_bandwidth_mbps
                .or(attachment.capacity_mbps)
                .unwrap_or(0);
            let upload = attachment
                .upload_bandwidth_mbps
                .or(attachment.capacity_mbps)
                .unwrap_or(0);
            node.insert(
                "downloadBandwidthMbps".to_string(),
                Value::Number(download.into()),
            );
            node.insert(
                "uploadBandwidthMbps".to_string(),
                Value::Number(upload.into()),
            );
            node.insert(
                "name".to_string(),
                Value::String(attachment.attachment_name.clone()),
            );
            return true;
        }
        if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut)
            && update_node_bandwidths_by_id(children, node_id, attachment)
        {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, Debug, Default)]
struct CompiledRatePair {
    download: Option<u64>,
    upload: Option<u64>,
}

fn compiled_rate_pair(download: Option<u64>, upload: Option<u64>) -> CompiledRatePair {
    CompiledRatePair { download, upload }
}
