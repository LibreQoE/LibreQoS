fn load_runtime_shaping_overrides(config: &Config) -> Result<lqos_overrides::OverrideFile> {
    let apply_stormguard = config
        .stormguard
        .as_ref()
        .is_some_and(|stormguard| stormguard.enabled && !stormguard.dry_run);
    let apply_treeguard = config.treeguard.enabled;
    OverrideStore::load_effective_for_config(config, apply_stormguard, apply_treeguard)
        .with_context(|| "Unable to load effective override layers")
}

#[derive(Default)]
struct QueueVirtualizationContext {
    direct_circuit_node_ids: HashSet<String>,
    direct_circuit_node_names: HashSet<String>,
    forced_visible_node_names: HashSet<String>,
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

fn forced_visible_node_names(overrides: &lqos_overrides::OverrideFile) -> HashSet<String> {
    overrides
        .network_adjustments()
        .iter()
        .filter_map(|adjustment| match adjustment {
            NetworkAdjustment::SetNodeVirtual {
                node_name,
                virtual_node: false,
            } => Some(node_name.trim()),
            _ => None,
        })
        .filter(|node_name| !node_name.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn load_queue_virtualization_context(
    config: &Config,
    ui_state: &TopologyEditorStateFile,
) -> Result<QueueVirtualizationContext> {
    let runtime_overrides = load_runtime_shaping_overrides(config)?;
    let attachment_owner_by_attachment_id = build_attachment_owner_map(ui_state);
    let (direct_circuit_node_ids, direct_circuit_node_names) =
        if topology_import_ingress_enabled(config) {
            let Some((mut shaped_devices, circuit_anchors)) =
                load_integration_shaping_artifacts(config)?
            else {
                return Ok(QueueVirtualizationContext {
                    forced_visible_node_names: forced_visible_node_names(&runtime_overrides),
                    ..QueueVirtualizationContext::default()
                });
            };
            let runtime_devices = apply_runtime_shaped_device_overrides(
                std::mem::take(&mut shaped_devices.devices),
                &runtime_overrides,
            );
            shaped_devices.replace_with_new_data(runtime_devices);
            (
                collect_direct_circuit_node_ids(
                    &shaped_devices,
                    &circuit_anchors,
                    &attachment_owner_by_attachment_id,
                ),
                collect_direct_circuit_node_names(&shaped_devices, &circuit_anchors),
            )
        } else {
            let shaped_devices_path = ConfigShapedDevices::path_for_config(config);
            if !shaped_devices_path.exists() {
                (HashSet::new(), HashSet::new())
            } else {
                let shaped_devices_mtime = std::fs::metadata(&shaped_devices_path)
                    .ok()
                    .and_then(|metadata| metadata.modified().ok());
                let shaped_devices = ConfigShapedDevices::load_for_config(config).with_context(
                    || "Unable to load ShapedDevices.csv while preparing queue virtualization",
                )?;
                let circuit_anchors = load_circuit_anchors(config, shaped_devices_mtime);
                (
                    collect_direct_circuit_node_ids(
                        &shaped_devices,
                        &circuit_anchors,
                        &attachment_owner_by_attachment_id,
                    ),
                    collect_direct_circuit_node_names(&shaped_devices, &circuit_anchors),
                )
            }
        };

    Ok(QueueVirtualizationContext {
        direct_circuit_node_ids,
        direct_circuit_node_names,
        forced_visible_node_names: forced_visible_node_names(&runtime_overrides),
    })
}
