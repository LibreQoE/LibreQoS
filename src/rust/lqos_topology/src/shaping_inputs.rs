fn collect_exported_effective_nodes(
    value: &Value,
    by_id: &mut HashMap<String, String>,
    unique_names: &mut HashMap<String, Option<String>>,
    ambiguous_names: &mut HashSet<String>,
) {
    let Some(nodes) = value.as_object() else {
        return;
    };
    for (key, node) in nodes {
        let Some(map) = node.as_object() else {
            continue;
        };
        let is_virtual = map.get("virtual").and_then(Value::as_bool).unwrap_or(false);
        let node_id = map
            .get("id")
            .and_then(Value::as_str)
            .and_then(optional_non_empty);
        let node_name = map
            .get("name")
            .and_then(Value::as_str)
            .and_then(optional_non_empty)
            .or_else(|| optional_non_empty(key));
        if !is_virtual && let Some(node_name) = node_name {
            if let Some(node_id) = node_id.clone() {
                by_id.insert(node_id, node_name.clone());
            }
            if !ambiguous_names.contains(&node_name) {
                match unique_names.entry(node_name) {
                    Entry::Vacant(entry) => {
                        entry.insert(node_id);
                    }
                    Entry::Occupied(entry) => {
                        let (duplicate_name, _) = entry.remove_entry();
                        ambiguous_names.insert(duplicate_name);
                    }
                }
            }
        }
        if let Some(children) = map.get("children") {
            collect_exported_effective_nodes(children, by_id, unique_names, ambiguous_names);
        }
    }
}

fn collect_exported_effective_aliases(
    value: &Value,
    aliases: &mut HashMap<String, (String, String)>,
) {
    let Some(nodes) = value.as_object() else {
        return;
    };
    for (key, node) in nodes {
        let Some(map) = node.as_object() else {
            continue;
        };
        let is_virtual = map.get("virtual").and_then(Value::as_bool).unwrap_or(false);
        let node_id = map
            .get("id")
            .and_then(Value::as_str)
            .and_then(optional_non_empty);
        let node_name = map
            .get("name")
            .and_then(Value::as_str)
            .and_then(optional_non_empty)
            .or_else(|| optional_non_empty(key));
        let active_attachment_name = map
            .get("active_attachment_name")
            .and_then(Value::as_str)
            .and_then(optional_non_empty);
        if !is_virtual
            && let (Some(alias), Some(node_id), Some(node_name)) =
                (active_attachment_name, node_id, node_name)
        {
            aliases.entry(alias).or_insert((node_id, node_name));
        }
        if let Some(children) = map.get("children") {
            collect_exported_effective_aliases(children, aliases);
        }
    }
}

fn build_effective_queue_aliases(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    exported_effective_nodes: &HashMap<String, String>,
) -> (EffectiveQueueAliasMap, EffectiveQueueAliasMap) {
    let ui_by_node = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let logical_parent_by_node = effective
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node.logical_parent_node_id.as_str()))
        .collect::<HashMap<_, _>>();
    let mut aliases_by_id = HashMap::new();
    let mut aliases_by_name = HashMap::new();

    for effective_node in &effective.nodes {
        if exported_effective_nodes.contains_key(&effective_node.node_id) {
            continue;
        }
        let Some(resolved) = nearest_exported_effective_parent(
            &effective_node.logical_parent_node_id,
            &logical_parent_by_node,
            exported_effective_nodes,
        ) else {
            continue;
        };
        aliases_by_id
            .entry(effective_node.node_id.clone())
            .or_insert_with(|| resolved.clone());
        if let Some(ui_node) = ui_by_node.get(effective_node.node_id.as_str()).copied() {
            aliases_by_name
                .entry(ui_node.node_name.clone())
                .or_insert_with(|| resolved.clone());
        }
    }

    (aliases_by_id, aliases_by_name)
}

fn nearest_exported_effective_parent(
    parent_id: &str,
    logical_parent_by_node: &HashMap<&str, &str>,
    exported_effective_nodes: &HashMap<String, String>,
) -> Option<(String, String)> {
    let mut cursor = parent_id.trim();
    let mut seen = HashSet::new();
    while !cursor.is_empty() && seen.insert(cursor.to_string()) {
        if let Some(parent_name) = exported_effective_nodes.get(cursor) {
            return Some((cursor.to_string(), parent_name.clone()));
        }
        cursor = logical_parent_by_node
            .get(cursor)
            .copied()
            .unwrap_or_default();
    }
    None
}

fn effective_parent_is_exported(
    parent_id: &str,
    parent_name: &str,
    exported_effective_nodes: &HashMap<String, String>,
    exported_effective_names: &HashMap<String, Option<String>>,
) -> bool {
    optional_non_empty(parent_id)
        .is_some_and(|node_id| exported_effective_nodes.contains_key(&node_id))
        || optional_non_empty(parent_name)
            .is_some_and(|node_name| exported_effective_names.contains_key(&node_name))
}

fn resolve_legacy_parent_from_effective_tree(
    parent_node: &str,
    parent_node_id: Option<&str>,
    exported_effective_nodes: &HashMap<String, String>,
    exported_effective_names: &HashMap<String, Option<String>>,
    exported_effective_aliases: &HashMap<String, (String, String)>,
    queue_aliases_by_id: &HashMap<String, (String, String)>,
    queue_aliases_by_name: &HashMap<String, (String, String)>,
) -> Option<(String, String)> {
    let trimmed_id = parent_node_id.and_then(optional_non_empty);
    let trimmed_name = optional_non_empty(parent_node);

    if let Some(parent_id) = trimmed_id.as_deref()
        && let Some(parent_name) = exported_effective_nodes.get(parent_id).cloned()
    {
        return Some((parent_id.to_string(), parent_name));
    }
    if let Some(parent_id) = trimmed_id.as_deref()
        && let Some(resolved) = queue_aliases_by_id.get(parent_id).cloned()
    {
        return Some(resolved);
    }
    if let Some(parent_name) = trimmed_name.as_deref()
        && let Some(parent_id) = exported_effective_names.get(parent_name)
    {
        return Some((
            parent_id.clone().unwrap_or_default(),
            parent_name.to_string(),
        ));
    }
    if let Some(parent_name) = trimmed_name.as_deref()
        && let Some(resolved) = queue_aliases_by_name.get(parent_name).cloned()
    {
        return Some(resolved);
    }
    trimmed_name.and_then(|alias| exported_effective_aliases.get(&alias).cloned())
}

fn selected_attachment_name_for_node(
    ui_node: &TopologyEditorNode,
    effective_node: &TopologyEffectiveNodeState,
) -> Option<String> {
    effective_node
        .effective_attachment_id
        .as_deref()
        .and_then(|attachment_id| {
            ui_node
                .allowed_parents
                .iter()
                .find_map(|parent| option_name(parent, attachment_id))
        })
        .or_else(|| optional_non_empty_owned(ui_node.effective_attachment_name.clone()))
        .or_else(|| optional_non_empty_owned(ui_node.current_attachment_name.clone()))
}

fn build_attachment_owner_map(
    ui_state: &TopologyEditorStateFile,
) -> HashMap<String, (String, String)> {
    let mut owners = HashMap::new();
    for node in &ui_state.nodes {
        for parent in &node.allowed_parents {
            for option in &parent.attachment_options {
                let Some(attachment_id) = optional_non_empty(&option.attachment_id) else {
                    continue;
                };
                if attachment_id == TOPOLOGY_ATTACHMENT_AUTO_ID {
                    continue;
                }
                owners
                    .entry(attachment_id)
                    .or_insert_with(|| (node.node_id.clone(), node.node_name.clone()));
            }
        }
    }
    owners
}

fn resolve_effective_parent_from_anchor(
    anchor_id: &str,
    ui_by_node: &HashMap<&str, &TopologyEditorNode>,
    effective_by_node: &HashMap<&str, &TopologyEffectiveNodeState>,
    exported_effective_nodes: &HashMap<String, String>,
    attachment_owner_by_attachment_id: &HashMap<String, (String, String)>,
    queue_aliases_by_id: &HashMap<String, (String, String)>,
) -> Option<(String, String, Option<String>, Option<String>)> {
    let anchor_id = anchor_id.trim();
    if anchor_id.is_empty() {
        return None;
    }

    if let Some(parent_name) = exported_effective_nodes.get(anchor_id).cloned() {
        let attachment_id = effective_by_node
            .get(anchor_id)
            .and_then(|node| optional_non_empty_owned(node.effective_attachment_id.clone()));
        let attachment_name = ui_by_node.get(anchor_id).and_then(|ui_node| {
            effective_by_node.get(anchor_id).and_then(|effective_node| {
                selected_attachment_name_for_node(ui_node, effective_node)
            })
        });
        return Some((
            anchor_id.to_string(),
            parent_name,
            attachment_id,
            attachment_name,
        ));
    }
    let Some((owner_node_id, _owner_node_name)) =
        attachment_owner_by_attachment_id.get(anchor_id).cloned()
    else {
        if let Some((parent_id, parent_name)) = queue_aliases_by_id.get(anchor_id).cloned() {
            return Some((parent_id, parent_name, None, None));
        }
        return None;
    };
    let owner_ui = ui_by_node.get(owner_node_id.as_str()).copied()?;
    let owner_effective = effective_by_node.get(owner_node_id.as_str()).copied()?;

    if let Some(selected_attachment_id) = owner_effective.effective_attachment_id.as_deref()
        && let Some(parent_name) = exported_effective_nodes
            .get(selected_attachment_id)
            .cloned()
    {
        return Some((
            selected_attachment_id.to_string(),
            parent_name,
            optional_non_empty(selected_attachment_id),
            selected_attachment_name_for_node(owner_ui, owner_effective),
        ));
    }

    if let Some(parent_name) = exported_effective_nodes
        .get(owner_node_id.as_str())
        .cloned()
    {
        return Some((
            owner_node_id,
            parent_name,
            optional_non_empty_owned(owner_effective.effective_attachment_id.clone()),
            selected_attachment_name_for_node(owner_ui, owner_effective),
        ));
    }
    if let Some((parent_id, parent_name)) = queue_aliases_by_id.get(owner_node_id.as_str()).cloned()
    {
        return Some((
            parent_id,
            parent_name,
            optional_non_empty_owned(owner_effective.effective_attachment_id.clone()),
            selected_attachment_name_for_node(owner_ui, owner_effective),
        ));
    }

    None
}

fn duplicate_circuit_shape_conflicts(
    circuit: &TopologyShapingCircuitInput,
    device: &lqos_config::ShapedDevice,
) -> Vec<&'static str> {
    let mut conflicts = Vec::new();
    if circuit.download_min_mbps != device.download_min_mbps {
        conflicts.push("Download Min Mbps");
    }
    if circuit.upload_min_mbps != device.upload_min_mbps {
        conflicts.push("Upload Min Mbps");
    }
    if circuit.download_max_mbps != device.download_max_mbps {
        conflicts.push("Download Max Mbps");
    }
    if circuit.upload_max_mbps != device.upload_max_mbps {
        conflicts.push("Upload Max Mbps");
    }
    if circuit.comment != device.comment {
        conflicts.push("Comment");
    }
    if circuit.sqm_override != device.sqm_override {
        conflicts.push("sqm");
    }
    conflicts
}

fn ipv4_with_prefix_to_string(entry: &(std::net::Ipv4Addr, u32)) -> String {
    if entry.1 >= 32 {
        entry.0.to_string()
    } else {
        format!("{}/{}", entry.0, entry.1)
    }
}

fn ipv6_with_prefix_to_string(entry: &(std::net::Ipv6Addr, u32)) -> String {
    if entry.1 >= 128 {
        entry.0.to_string()
    } else {
        format!("{}/{}", entry.0, entry.1)
    }
}

fn load_circuit_anchors(
    config: &Config,
    shaped_devices_mtime: Option<std::time::SystemTime>,
) -> Vec<CircuitAnchor> {
    let anchors_path = circuit_anchors_path(config);
    let anchors_metadata = std::fs::metadata(&anchors_path).ok();
    if let Some(shaped_devices_mtime) = shaped_devices_mtime
        && let Some(anchors_mtime) = anchors_metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok())
        && anchors_mtime < shaped_devices_mtime
    {
        return Vec::new();
    }

    CircuitAnchorsFile::load(config)
        .map(|file| file.anchors)
        .unwrap_or_default()
}

fn load_integration_shaping_artifacts(
    config: &Config,
) -> Result<Option<(ConfigShapedDevices, Vec<CircuitAnchor>)>> {
    let topology_import = TopologyImportFile::load(config)
        .with_context(|| "Unable to load topology_import.json while validating shaping ingress")?;
    let Some(compiled_shaping) = TopologyCompiledShapingFile::load(config).with_context(
        || "Unable to load topology_compiled_shaping.json while building shaping_inputs.json",
    )?
    else {
        return Ok(None);
    };
    let import_identity = topology_import
        .as_ref()
        .and_then(|file| file.ingress_identity.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let compiled_identity = compiled_shaping
        .ingress_identity
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let (Some(import_identity), Some(compiled_identity)) = (import_identity, compiled_identity)
        && import_identity != compiled_identity
    {
        anyhow::bail!(
            "Integration compiled shaping ingress identity '{}' did not match topology import identity '{}'",
            compiled_identity,
            import_identity
        );
    }
    Ok(Some(compiled_shaping.shaping_artifacts()))
}

fn build_shaping_inputs(
    config: &Config,
    artifacts: &EffectiveTopologyArtifacts,
) -> Result<Option<TopologyShapingInputsFile>> {
    let integration_ingress = topology_import_ingress_enabled(config);
    let (mut shaped_devices, circuit_anchor_rows) = if integration_ingress {
        let Some((shaped_devices, circuit_anchors)) = load_integration_shaping_artifacts(config)?
        else {
            return Ok(None);
        };
        (shaped_devices, circuit_anchors)
    } else {
        let shaped_devices_path = ConfigShapedDevices::path_for_config(config);
        if !shaped_devices_path.exists() {
            return Ok(None);
        }

        let shaped_devices_mtime = std::fs::metadata(&shaped_devices_path)
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        (
            ConfigShapedDevices::load_for_config(config).with_context(
                || "Unable to load ShapedDevices.csv while building shaping_inputs.json",
            )?,
            load_circuit_anchors(config, shaped_devices_mtime),
        )
    };
    if integration_ingress {
        let effective_overrides = load_runtime_shaping_overrides(config).with_context(
            || "Unable to load effective overrides while building shaping_inputs.json",
        )?;
        let runtime_devices = apply_runtime_shaped_device_overrides(
            shaped_devices.devices.clone(),
            &effective_overrides,
        );
        shaped_devices.replace_with_new_data(runtime_devices);
    }
    let flat_bucket_assignments = if runtime_flat_mode(config) {
        Some(build_flat_bucket_assignments(
            config,
            &shaped_devices.devices,
        ))
    } else {
        None
    };
    let circuit_anchors = circuit_anchor_rows
        .into_iter()
        .map(|anchor| (anchor.circuit_id.clone(), anchor))
        .collect::<HashMap<_, _>>();
    let effective_by_node = artifacts
        .effective
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let ui_by_node = artifacts
        .ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut exported_effective_nodes = HashMap::<String, String>::new();
    let mut exported_effective_names = HashMap::<String, Option<String>>::new();
    let mut exported_effective_aliases = HashMap::<String, (String, String)>::new();
    let mut ambiguous_exported_effective_names = HashSet::<String>::new();
    let attachment_owner_by_attachment_id = build_attachment_owner_map(&artifacts.ui_state);
    if let Some(effective_network) = artifacts.effective_network.as_ref() {
        collect_exported_effective_nodes(
            effective_network,
            &mut exported_effective_nodes,
            &mut exported_effective_names,
            &mut ambiguous_exported_effective_names,
        );
        collect_exported_effective_aliases(effective_network, &mut exported_effective_aliases);
    }
    let (queue_aliases_by_id, queue_aliases_by_name) = build_effective_queue_aliases(
        &artifacts.ui_state,
        &artifacts.effective,
        &exported_effective_nodes,
    );

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut fallback_warning_summary = TopologyFallbackWarningSummary::default();
    let mut circuits = Vec::<TopologyShapingCircuitInput>::new();
    let mut circuits_by_id = HashMap::<String, usize>::new();

    for device in &shaped_devices.devices {
        let anchor_from_file = circuit_anchors.get(&device.circuit_id);
        let anchor_node_id = anchor_from_file
            .map(|anchor| anchor.anchor_node_id.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| optional_non_empty_owned(device.anchor_node_id.clone()));
        let mut anchor_node_name = anchor_from_file.and_then(|anchor| {
            anchor
                .anchor_node_name
                .as_ref()
                .and_then(|value| optional_non_empty(value))
        });
        let (
            mut effective_parent_node_id,
            mut effective_parent_node_name,
            mut effective_attachment_id,
            mut effective_attachment_name,
            mut resolution_source,
        ) = if let Some((bucket_id, bucket_name)) = flat_bucket_assignments
            .as_ref()
            .and_then(|assignments| assignments.get(&device.circuit_id))
        {
            (
                bucket_id.clone(),
                bucket_name.clone(),
                None,
                None,
                TopologyShapingResolutionSource::FlatBucket,
            )
        } else if let Some(anchor_id) = anchor_node_id.as_deref() {
            match resolve_effective_parent_from_anchor(
                anchor_id,
                &ui_by_node,
                &effective_by_node,
                &exported_effective_nodes,
                &attachment_owner_by_attachment_id,
                &queue_aliases_by_id,
            ) {
                Some((
                    resolved_parent_id,
                    resolved_parent_name,
                    resolved_attachment_id,
                    resolved_attachment_name,
                )) => {
                    if let Some(ui_node) = ui_by_node.get(anchor_id) {
                        anchor_node_name = Some(ui_node.node_name.clone());
                    }
                    (
                        resolved_parent_id,
                        resolved_parent_name,
                        resolved_attachment_id,
                        resolved_attachment_name,
                        TopologyShapingResolutionSource::TopologyAnchor,
                    )
                }
                None => match (
                    ui_by_node.get(anchor_id),
                    effective_by_node.get(anchor_id),
                    anchor_from_file,
                ) {
                    (Some(ui_node), Some(_effective_node), _) => {
                        anchor_node_name = Some(ui_node.node_name.clone());
                        fallback_warning_summary.record_unresolved_exported_anchor(
                            &device.circuit_id,
                            anchor_id,
                            &ui_node.node_name,
                        );
                        (
                            String::new(),
                            String::new(),
                            None,
                            None,
                            TopologyShapingResolutionSource::RuntimeFallback,
                        )
                    }
                    (None, None, Some(anchor)) => {
                        fallback_warning_summary.record_missing_anchor(
                            &device.circuit_id,
                            anchor_id,
                            anchor.anchor_node_name.as_deref(),
                        );
                        (
                            String::new(),
                            String::new(),
                            None,
                            None,
                            TopologyShapingResolutionSource::RuntimeFallback,
                        )
                    }
                    _ => {
                        fallback_warning_summary.record_missing_anchor(
                            &device.circuit_id,
                            anchor_id,
                            None,
                        );
                        (
                            String::new(),
                            String::new(),
                            None,
                            None,
                            TopologyShapingResolutionSource::RuntimeFallback,
                        )
                    }
                },
            }
        } else if let Some((resolved_parent_id, resolved_parent_name)) =
            resolve_legacy_parent_from_effective_tree(
                &device.parent_node,
                device.parent_node_id.as_deref(),
                &exported_effective_nodes,
                &exported_effective_names,
                &exported_effective_aliases,
                &queue_aliases_by_id,
                &queue_aliases_by_name,
            )
        {
            (
                resolved_parent_id,
                resolved_parent_name,
                None,
                None,
                TopologyShapingResolutionSource::LegacyParent,
            )
        } else {
            if optional_non_empty(&device.parent_node).is_some()
                || optional_non_empty_owned(device.parent_node_id.clone()).is_some()
            {
                fallback_warning_summary.record_missing_parent(
                    &device.circuit_id,
                    device.parent_node.trim(),
                    device.parent_node_id.clone().unwrap_or_default().trim(),
                );
            }
            (
                String::new(),
                String::new(),
                None,
                None,
                TopologyShapingResolutionSource::RuntimeFallback,
            )
        };

        if matches!(
            resolution_source,
            TopologyShapingResolutionSource::TopologyAnchor
                | TopologyShapingResolutionSource::LegacyParent
        ) && !effective_parent_is_exported(
            &effective_parent_node_id,
            &effective_parent_node_name,
            &exported_effective_nodes,
            &exported_effective_names,
        ) {
            fallback_warning_summary.record_non_exported_parent(
                &device.circuit_id,
                &effective_parent_node_name,
                &effective_parent_node_id,
            );
            effective_parent_node_id.clear();
            effective_parent_node_name.clear();
            effective_attachment_id = None;
            effective_attachment_name = None;
            resolution_source = TopologyShapingResolutionSource::RuntimeFallback;
        }

        let logical_parent_node_name = optional_non_empty(&device.parent_node);
        let logical_parent_node_id = optional_non_empty_owned(device.parent_node_id.clone());
        let circuit_index = if let Some(index) = circuits_by_id.get(&device.circuit_id).copied() {
            let circuit = &mut circuits[index];
            if circuit.anchor_node_id != anchor_node_id {
                errors.push(format!(
                    "Circuit '{}' had multiple AnchorNodeID values while building shaping inputs.",
                    device.circuit_id
                ));
            }
            if circuit.effective_parent_node_id != effective_parent_node_id
                || circuit.effective_parent_node_name != effective_parent_node_name
            {
                errors.push(format!(
                    "Circuit '{}' resolved to multiple effective parents while building shaping_inputs.json.",
                    device.circuit_id
                ));
            }
            let conflicts = duplicate_circuit_shape_conflicts(circuit, device);
            if !conflicts.is_empty() {
                errors.push(format!(
                    "Circuit '{}' had conflicting circuit-level fields across ShapedDevices.csv rows while building shaping_inputs.json: {}.",
                    device.circuit_id,
                    conflicts.join(", ")
                ));
            }
            index
        } else {
            let index = circuits.len();
            circuits_by_id.insert(device.circuit_id.clone(), index);
            circuits.push(TopologyShapingCircuitInput {
                circuit_id: device.circuit_id.clone(),
                circuit_name: device.circuit_name.clone(),
                anchor_node_id: anchor_node_id.clone(),
                anchor_node_name,
                logical_parent_node_name,
                logical_parent_node_id,
                effective_parent_node_name,
                effective_parent_node_id,
                effective_attachment_id,
                effective_attachment_name,
                resolution_source,
                download_min_mbps: device.download_min_mbps,
                upload_min_mbps: device.upload_min_mbps,
                download_max_mbps: device.download_max_mbps,
                upload_max_mbps: device.upload_max_mbps,
                comment: device.comment.clone(),
                sqm_override: device.sqm_override.clone(),
                devices: Vec::new(),
            });
            index
        };

        circuits[circuit_index]
            .devices
            .push(TopologyShapingDeviceInput {
                device_id: device.device_id.clone(),
                device_name: device.device_name.clone(),
                mac: device.mac.clone(),
                ipv4: device.ipv4.iter().map(ipv4_with_prefix_to_string).collect(),
                ipv6: device.ipv6.iter().map(ipv6_with_prefix_to_string).collect(),
                comment: device.comment.clone(),
            });
    }

    circuits.sort_unstable_by(|left, right| left.circuit_id.cmp(&right.circuit_id));
    for circuit in &mut circuits {
        circuit
            .devices
            .sort_unstable_by(|left, right| left.device_id.cmp(&right.device_id));
    }
    fallback_warning_summary.append_to(&mut warnings);

    let unresolved_runtime_fallbacks = circuits
        .iter()
        .filter(|circuit| {
            circuit.effective_parent_node_id.trim().is_empty()
                && circuit.resolution_source == TopologyShapingResolutionSource::RuntimeFallback
        })
        .count();
    if unresolved_runtime_fallbacks > 0 {
        if config.shared_topology_compile_mode() == Some("flat") {
            warnings.push(format!(
                "Flat topology mode assigned {unresolved_runtime_fallbacks} circuit(s) to generated parent nodes during queue construction."
            ));
        } else {
            push_unresolved_runtime_fallback_summary(&mut warnings, unresolved_runtime_fallbacks);
        }
    }

    if !errors.is_empty() {
        let mut message = String::from(
            "Unable to build shaping_inputs.json due to runtime topology contract errors:",
        );
        for error in errors {
            message.push_str("\n- ");
            message.push_str(&error);
        }
        return Err(anyhow::anyhow!(message));
    }

    Ok(Some(TopologyShapingInputsFile {
        schema_version: 1,
        shaping_generation: String::new(),
        generated_unix: now_unix(),
        canonical_generated_unix: artifacts.effective.canonical_generated_unix,
        effective_generated_unix: artifacts.effective.generated_unix,
        warnings,
        circuits,
    }))
}

fn apply_runtime_shaped_device_overrides(
    base_devices: Vec<lqos_config::ShapedDevice>,
    overrides: &lqos_overrides::OverrideFile,
) -> Vec<lqos_config::ShapedDevice> {
    let mut devices = base_devices;
    for override_device in overrides.persistent_devices() {
        if let Some(existing_index) = devices
            .iter()
            .position(|device| device.device_id == override_device.device_id)
        {
            devices[existing_index] = override_device.clone();
        } else {
            devices.push(override_device.clone());
        }
    }

    for adjustment in overrides.circuit_adjustments() {
        match adjustment {
            CircuitAdjustment::CircuitAdjustSpeed {
                circuit_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.circuit_id == *circuit_id)
                {
                    if let Some(value) = min_download_bandwidth {
                        device.download_min_mbps = *value;
                    }
                    if let Some(value) = max_download_bandwidth {
                        device.download_max_mbps = *value;
                    }
                    if let Some(value) = min_upload_bandwidth {
                        device.upload_min_mbps = *value;
                    }
                    if let Some(value) = max_upload_bandwidth {
                        device.upload_max_mbps = *value;
                    }
                }
            }
            CircuitAdjustment::DeviceAdjustSpeed {
                device_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.device_id == *device_id)
                {
                    if let Some(value) = min_download_bandwidth {
                        device.download_min_mbps = *value;
                    }
                    if let Some(value) = max_download_bandwidth {
                        device.download_max_mbps = *value;
                    }
                    if let Some(value) = min_upload_bandwidth {
                        device.upload_min_mbps = *value;
                    }
                    if let Some(value) = max_upload_bandwidth {
                        device.upload_max_mbps = *value;
                    }
                }
            }
            CircuitAdjustment::DeviceAdjustSqm {
                device_id,
                sqm_override,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.device_id == *device_id)
                {
                    device.sqm_override = sqm_override
                        .as_ref()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty());
                }
            }
            CircuitAdjustment::RemoveCircuit { circuit_id } => {
                devices.retain(|device| device.circuit_id != *circuit_id);
            }
            CircuitAdjustment::RemoveDevice { device_id } => {
                devices.retain(|device| device.device_id != *device_id);
            }
            CircuitAdjustment::ReparentCircuit {
                circuit_id,
                parent_node,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.circuit_id == *circuit_id)
                {
                    device.parent_node = parent_node.clone();
                    device.parent_node_id = None;
                    device.anchor_node_id = None;
                }
            }
        }
    }

    devices
}
