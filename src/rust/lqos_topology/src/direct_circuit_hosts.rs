fn load_runtime_shaping_overrides(
    config: &Config,
    read_only: bool,
) -> Result<lqos_overrides::OverrideFile> {
    let apply_stormguard = config
        .stormguard
        .as_ref()
        .is_some_and(|stormguard| stormguard.enabled && !stormguard.dry_run);
    let loaded = if read_only {
        OverrideStore::load_effective_for_config_read_only(config, apply_stormguard)
    } else {
        OverrideStore::load_effective_for_config(config, apply_stormguard)
    };
    loaded.with_context(|| "Unable to load effective override layers")
}

#[derive(Default)]
struct DirectCircuitHosts {
    direct_circuit_node_ids: HashSet<String>,
    direct_circuit_node_names: HashSet<String>,
}

fn insert_direct_node_and_attachment_owner(
    target: &mut HashSet<String>,
    value: Option<&str>,
    attachment_owner_by_attachment_id: &HashMap<String, (String, String)>,
) {
    let Some(node_id) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    target.insert(node_id.to_string());
    if let Some((owner_node_id, _)) = attachment_owner_by_attachment_id.get(node_id) {
        target.insert(owner_node_id.clone());
    }
}

fn collect_direct_circuit_node_ids(
    shaped_devices: &ConfigShapedDevices,
    circuit_anchors: &[CircuitAnchor],
    attachment_owner_by_attachment_id: &HashMap<String, (String, String)>,
) -> HashSet<String> {
    let mut direct_node_ids = HashSet::new();
    for device in &shaped_devices.devices {
        insert_direct_node_and_attachment_owner(
            &mut direct_node_ids,
            device.parent_node_id.as_deref(),
            attachment_owner_by_attachment_id,
        );
        insert_direct_node_and_attachment_owner(
            &mut direct_node_ids,
            device.anchor_node_id.as_deref(),
            attachment_owner_by_attachment_id,
        );
    }
    for anchor in circuit_anchors {
        insert_direct_node_and_attachment_owner(
            &mut direct_node_ids,
            Some(anchor.anchor_node_id.as_str()),
            attachment_owner_by_attachment_id,
        );
    }
    direct_node_ids
}

fn insert_direct_node_name(target: &mut HashSet<String>, value: Option<&str>) {
    let Some(node_name) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    target.insert(node_name.to_string());
}

fn collect_direct_circuit_node_names(
    shaped_devices: &ConfigShapedDevices,
    circuit_anchors: &[CircuitAnchor],
) -> HashSet<String> {
    let mut direct_node_names = HashSet::new();
    for device in &shaped_devices.devices {
        insert_direct_node_name(&mut direct_node_names, Some(device.parent_node.as_str()));
    }
    for anchor in circuit_anchors {
        insert_direct_node_name(&mut direct_node_names, anchor.anchor_node_name.as_deref());
    }
    direct_node_names
}

fn load_direct_circuit_hosts(
    config: &Config,
    ui_state: &TopologyEditorStateFile,
    read_only: bool,
) -> Result<DirectCircuitHosts> {
    let runtime_overrides = load_runtime_shaping_overrides(config, read_only)?;
    let attachment_owner_by_attachment_id = build_attachment_owner_map(ui_state);
    // Phase C S4 (producer unification): read the compiled import only.
    let Some((mut shaped_devices, circuit_anchors, _match_index)) =
        load_integration_shaping_artifacts(config)?
    else {
        return Ok(DirectCircuitHosts::default());
    };
    let runtime_devices = apply_runtime_shaped_device_overrides(
        std::mem::take(&mut shaped_devices.devices),
        &runtime_overrides,
    );
    shaped_devices.replace_with_new_data(runtime_devices);
    let direct_circuit_node_ids = collect_direct_circuit_node_ids(
        &shaped_devices,
        &circuit_anchors,
        &attachment_owner_by_attachment_id,
    );
    let direct_circuit_node_names =
        collect_direct_circuit_node_names(&shaped_devices, &circuit_anchors);

    Ok(DirectCircuitHosts {
        direct_circuit_node_ids,
        direct_circuit_node_names,
    })
}
