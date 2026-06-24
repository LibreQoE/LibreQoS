fn rate_pair_from_attachment(attachment: &TopologyAttachmentOption) -> CompiledRatePair {
    compiled_rate_pair(
        attachment
            .download_bandwidth_mbps
            .or(attachment.capacity_mbps),
        attachment
            .upload_bandwidth_mbps
            .or(attachment.capacity_mbps),
    )
}

fn rate_pair_from_value(node: &Map<String, Value>) -> CompiledRatePair {
    compiled_rate_pair(
        node.get("downloadBandwidthMbps")
            .and_then(|value| match value {
                Value::Number(number) => number
                    .as_u64()
                    .or_else(|| number.as_f64().map(|value| value.round() as u64)),
                _ => None,
            }),
        node.get("uploadBandwidthMbps")
            .and_then(|value| match value {
                Value::Number(number) => number
                    .as_u64()
                    .or_else(|| number.as_f64().map(|value| value.round() as u64)),
                _ => None,
            }),
    )
}

fn rate_pair_from_canonical_node(
    node: &TopologyCanonicalNode,
    use_compatibility_export_rates: bool,
) -> CompiledRatePair {
    if node.rate_input.source == TopologyCanonicalRateInputSource::CompatibilityExport
        && !use_compatibility_export_rates
    {
        return CompiledRatePair::default();
    }

    compiled_rate_pair(
        node.rate_input
            .intrinsic_download_mbps
            .or(node.rate_input.legacy_imported_download_mbps),
        node.rate_input
            .intrinsic_upload_mbps
            .or(node.rate_input.legacy_imported_upload_mbps),
    )
}

fn intersect_rate_pairs(base: CompiledRatePair, limit: CompiledRatePair) -> CompiledRatePair {
    let download = match (base.download, limit.download) {
        (Some(base), Some(limit)) => Some(base.min(limit)),
        (Some(base), None) => Some(base),
        (None, Some(limit)) => Some(limit),
        (None, None) => None,
    };
    let upload = match (base.upload, limit.upload) {
        (Some(base), Some(limit)) => Some(base.min(limit)),
        (Some(base), None) => Some(base),
        (None, Some(limit)) => Some(limit),
        (None, None) => None,
    };
    compiled_rate_pair(download, upload)
}

fn write_rate_pair(node: &mut Map<String, Value>, rates: CompiledRatePair) {
    if let Some(download) = rates.download {
        node.insert(
            "downloadBandwidthMbps".to_string(),
            Value::Number(download.into()),
        );
    }
    if let Some(upload) = rates.upload {
        node.insert(
            "uploadBandwidthMbps".to_string(),
            Value::Number(upload.into()),
        );
    }
}

fn selected_attachment_rate_caps(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> HashMap<String, CompiledRatePair> {
    let ui_nodes = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut caps = HashMap::new();
    for node in &effective.nodes {
        let Some(selected_attachment_id) = node.effective_attachment_id.as_deref() else {
            continue;
        };
        let Some(ui_node) = ui_nodes.get(node.node_id.as_str()).copied() else {
            continue;
        };
        let Some(parent) = ui_node
            .allowed_parents
            .iter()
            .find(|entry| entry.parent_node_id == node.logical_parent_node_id)
        else {
            continue;
        };
        let Some(attachment) = parent
            .attachment_options
            .iter()
            .find(|attachment| attachment.attachment_id == selected_attachment_id)
        else {
            continue;
        };
        caps.insert(node.node_id.clone(), rate_pair_from_attachment(attachment));
    }
    caps
}

fn recompile_effective_bandwidths_for_value(
    value: &mut Value,
    canonical_nodes: &HashMap<&str, &TopologyCanonicalNode>,
    selected_attachment_caps: &HashMap<String, CompiledRatePair>,
    inherited_parent_rates: Option<CompiledRatePair>,
    use_compatibility_export_rates: bool,
) {
    let Some(node) = value.as_object_mut() else {
        return;
    };
    let existing_rates = rate_pair_from_value(node);
    let node_id = node.get("id").and_then(Value::as_str);
    let canonical_node = node_id.and_then(|node_id| canonical_nodes.get(node_id).copied());
    let mut compiled = canonical_node
        .map(|node| rate_pair_from_canonical_node(node, use_compatibility_export_rates))
        .unwrap_or(existing_rates);
    let skip_existing_compatibility_rates = canonical_node.is_some_and(|node| {
        node.rate_input.source == TopologyCanonicalRateInputSource::CompatibilityExport
            && !use_compatibility_export_rates
    });
    if let Some(node_id) = node_id
        && let Some(attachment_rates) = selected_attachment_caps.get(node_id)
    {
        compiled = intersect_rate_pairs(compiled, *attachment_rates);
    }
    if let Some(parent_rates) = inherited_parent_rates {
        compiled = intersect_rate_pairs(compiled, parent_rates);
    }
    if compiled.download.is_none() {
        compiled.download = if skip_existing_compatibility_rates {
            inherited_parent_rates.and_then(|pair| pair.download)
        } else {
            existing_rates
                .download
                .or(inherited_parent_rates.and_then(|pair| pair.download))
        };
    }
    if compiled.upload.is_none() {
        compiled.upload = if skip_existing_compatibility_rates {
            inherited_parent_rates.and_then(|pair| pair.upload)
        } else {
            existing_rates
                .upload
                .or(inherited_parent_rates.and_then(|pair| pair.upload))
        };
    }
    write_rate_pair(node, compiled);
    let next_parent_rates = Some(rate_pair_from_value(node));
    if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut) {
        for child in children.values_mut() {
            recompile_effective_bandwidths_for_value(
                child,
                canonical_nodes,
                selected_attachment_caps,
                next_parent_rates,
                use_compatibility_export_rates,
            );
        }
    }
}

fn recompile_effective_network_bandwidths(
    root: &mut Map<String, Value>,
    canonical: &TopologyCanonicalStateFile,
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) {
    let canonical_nodes = canonical
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let selected_attachment_caps = selected_attachment_rate_caps(ui_state, effective);
    let use_compatibility_export_rates = canonical.ingress_kind
        != TopologyCanonicalIngressKind::NativeIntegration
        || canonical.source.starts_with("uisp/");
    for node in root.values_mut() {
        recompile_effective_bandwidths_for_value(
            node,
            &canonical_nodes,
            &selected_attachment_caps,
            None,
            use_compatibility_export_rates,
        );
    }
}
