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
    attachment_overrides: &HashMap<String, (u64, u64)>,
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
    // Part 5 (WO §5): an explicit, ACTIVE attachment override substitutes the
    // attachment/radio node's compiled rate so the squash's min (and the
    // inherited cap below) read the corrected value.
    if let Some((download, upload)) = node_id.and_then(|node_id| attachment_overrides.get(node_id)) {
        compiled = compiled_rate_pair(Some(*download), Some(*upload));
    }
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
                attachment_overrides,
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
    // Part 5 (WO §5): the ACTIVE attachment rate override corrects the
    // attachment/radio NODE's compiled rate, so the runtime squash's
    // `min_attachment_bandwidth` — and the descendant site's inherited cap —
    // run on the operator's value rather than the UISP telemetry.
    let attachment_overrides = active_attachment_rate_overrides(ui_state, effective);
    let use_compatibility_export_rates = canonical.ingress_kind
        != TopologyCanonicalIngressKind::NativeIntegration
        || canonical.source.starts_with("uisp/");
    for node in root.values_mut() {
        recompile_effective_bandwidths_for_value(
            node,
            &canonical_nodes,
            &selected_attachment_caps,
            &attachment_overrides,
            None,
            use_compatibility_export_rates,
        );
    }
}

/// Writes a site-speed override value as a JSON number, preserving whole-Mbps
/// integers and emitting a two-decimal fraction otherwise.
fn write_site_speed_override(node: &mut Map<String, Value>, key: &str, value: f32) {
    if !value.is_finite() || value < 0.0 {
        return;
    }
    let number = if value.fract().abs() < f32::EPSILON {
        serde_json::Number::from(value as u64)
    } else {
        let rounded = (value * 100.0).round() / 100.0;
        serde_json::Number::from_f64(f64::from(rounded))
            .unwrap_or_else(|| serde_json::Number::from(0))
    };
    node.insert(key.to_string(), Value::Number(number));
}

/// One resolved `AdjustSiteSpeed` override: `(node_id, site_name, download, upload)`.
type SiteSpeedOverride<'a> = (Option<&'a str>, &'a str, Option<f32>, Option<f32>);

/// K1b §4.6 pin 1: unix-seconds now, for lapsed-entry filtering in the fold.
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// K1b §4.1 item 1 (the site-rate fold): applies the MERGED override store's
/// `AdjustSiteSpeed` entries (operator + stormguard, operator-first) to the
/// effective tree's node ceils, so a StormGuard clamp produces an inner-op rate
/// delta the broker diff carries live.
///
/// Matching mirrors the UISP importer's `find_bandwidth_override`: an entry with
/// a non-empty `node_id` matches by id; otherwise it matches by node name. A
/// non-matching entry is inert (the detector picks sites from the queue
/// structure; a stale/missing target simply clamps nothing).
///
/// Must run AFTER `recompile_effective_network_bandwidths` (the canonical/
/// attachment recomputation would otherwise overwrite the clamp) and BEFORE
/// inner-op emission.
fn apply_site_speed_overrides(root: &mut Map<String, Value>, overrides: &[NetworkAdjustment]) {
    let now = now_unix_secs();
    let site_speeds = overrides
        .iter()
        .filter_map(|adjustment| match adjustment {
            NetworkAdjustment::AdjustSiteSpeed {
                node_id,
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
                expires_at_unix,
            } => {
                // K1b §4.6 pin 1 defense-in-depth: a lapsed clamp must not
                // apply even before the periodic sweep drops it from the file.
                if expires_at_unix.is_some_and(|expires| expires <= now) {
                    return None;
                }
                Some((
                    node_id.as_deref(),
                    site_name.as_str(),
                    *download_bandwidth_mbps,
                    *upload_bandwidth_mbps,
                ))
            }
            _ => None,
        })
        .collect::<Vec<SiteSpeedOverride<'_>>>();
    if site_speeds.is_empty() {
        return;
    }
    for (node_name, node) in root.iter_mut() {
        if let Some(node) = node.as_object_mut() {
            apply_site_speed_overrides_to_node(node_name, node, &site_speeds);
        }
    }
}

fn apply_site_speed_overrides_to_node(
    fallback_name: &str,
    node: &mut Map<String, Value>,
    site_speeds: &[SiteSpeedOverride<'_>],
) {
    let node_id = node.get("id").and_then(Value::as_str).map(str::to_string);
    // The effective compatibility tree keys nodes by name and does not
    // guarantee a `name` field (legacy network.json has none), so fall back to
    // the object key — the canonical name the operator/StormGuard overrides use.
    let node_name = node
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| fallback_name.to_string());
    for (override_id, override_name, download, upload) in site_speeds {
        let matches = match override_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(override_id) => node_id.as_deref() == Some(override_id),
            None => node_name == *override_name,
        };
        if !matches {
            continue;
        }
        if let Some(download) = download {
            write_site_speed_override(node, "downloadBandwidthMbps", *download);
        }
        if let Some(upload) = upload {
            write_site_speed_override(node, "uploadBandwidthMbps", *upload);
        }
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut) {
        for (child_name, child) in children.iter_mut() {
            if let Some(child) = child.as_object_mut() {
                apply_site_speed_overrides_to_node(child_name, child, site_speeds);
            }
        }
    }
}

// --- Attachment-rate override reach (WO §5 Part 5) --------------------------

/// Part 5 of the attachment-rate override (WO §5): the runtime squash's
/// `min_attachment_bandwidth` (`runtime_squash.rs`) reads the ATTACHMENT/RADIO
/// **node**'s `downloadBandwidthMbps`/`uploadBandwidthMbps` — not the attachment
/// **option**'s rate that parts 1-4 set. So an explicit, ACTIVE attachment
/// override must ALSO substitute the AP-hop node's rate. This collector feeds
/// [`recompile_effective_network_bandwidths`], which writes it as the node's
/// compiled ceil BEFORE `apply_runtime_squashing` consumes it; because recompile
/// runs before the squash, the squash's model is preserved (NO reorder) and the
/// corrected hop also propagates to the descendant site's inherited cap.
///
/// Conditionality (DRIVER, WO §6): the override applies IFF its attachment is
/// the site's ACTIVE/effective attachment; when the alternate is active the
/// override is inert (an operator-visible no-leak). AUTO attachments never
/// carry an override.
///
/// The option-level parts 1-4 already folded the store override onto
/// `has_rate_override` + the option's rates; reading those keeps this reach pass
/// store-agnostic (no `TopologyOverridesFile` plumbing) and automatically
/// excludes AUTO and inactive alternates.
fn active_attachment_rate_overrides(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
) -> HashMap<String, (u64, u64)> {
    let effective_by_node = effective
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut overrides = HashMap::new();
    for node in &ui_state.nodes {
        let Some(active_attachment_id) = effective_by_node
            .get(node.node_id.as_str())
            .and_then(|entry| entry.effective_attachment_id.as_deref())
        else {
            continue;
        };
        for parent in &node.allowed_parents {
            for option in &parent.attachment_options {
                if !option.has_rate_override || option.attachment_id != active_attachment_id {
                    continue;
                }
                let (Some(download), Some(upload)) = (
                    option.download_bandwidth_mbps,
                    option.upload_bandwidth_mbps,
                ) else {
                    continue;
                };
                overrides.insert(option.attachment_id.clone(), (download, upload));
            }
        }
    }
    overrides
}

// --- Node commit (guaranteed-rate) derivation -------------------------------

/// Recompiles each node's guaranteed commit (`*BandwidthMbpsMin`) into the
/// effective network tree.
///
/// Ports LibreQoS.py `findBandwidthMins` + `inheritBandwidthMaxes` +
/// `ensure_min_less_than_max`, minus the cross-field copy-paste bug: a node's
/// commit is `max(operator_min, Σ descendant circuit commit)`, capped by its
/// own ceil and its parent's commit, then clamped strictly below its ceil. The
/// effective network carries ceil only today, so this makes the commit the DPDK
/// HTB tree needs available in a single artifact.
///
/// Side effects: mutates `effective_network` in place.
pub fn recompile_effective_network_commits(
    effective_network: &mut Value,
    shaping_inputs: &TopologyShapingInputsFile,
) {
    let Some(root) = effective_network.as_object_mut() else {
        return;
    };

    // Aggregate each circuit's commit under its effective parent node id.
    let mut circuit_mins_by_node: HashMap<&str, (f64, f64)> = HashMap::new();
    for circuit in &shaping_inputs.circuits {
        let entry = circuit_mins_by_node
            .entry(circuit.effective_parent_node_id.as_str())
            .or_insert((0.0, 0.0));
        entry.0 += f64::from(circuit.download_min_mbps);
        entry.1 += f64::from(circuit.upload_min_mbps);
    }

    // Bottom-up: raw commit per node = sum of its subtree's circuit commits.
    let mut raw_by_node: HashMap<String, (f64, f64)> = HashMap::new();
    for node in root.values() {
        if let Some(node) = node.as_object() {
            collect_raw_node_commits(node, &circuit_mins_by_node, &mut raw_by_node);
        }
    }

    // Top-down: cap by ceil + parent commit, clamp below ceil, and write.
    for node in root.values_mut() {
        if let Some(node) = node.as_object_mut() {
            write_node_commits(node, &raw_by_node, f64::INFINITY, f64::INFINITY);
        }
    }
}

/// Recursively sums descendant circuit commits into `raw_by_node`.
///
/// Mirrors `findBandwidthMins`: returns the raw subtree sum so a parent can
/// aggregate its children, while recording each node's own raw sum.
fn collect_raw_node_commits(
    node: &Map<String, Value>,
    circuit_mins_by_node: &HashMap<&str, (f64, f64)>,
    raw_by_node: &mut HashMap<String, (f64, f64)>,
) -> (f64, f64) {
    let node_id = node.get("id").and_then(Value::as_str);
    let (mut dl, mut ul) = node_id
        .and_then(|id| circuit_mins_by_node.get(id).copied())
        .unwrap_or((0.0, 0.0));
    if let Some(children) = node.get("children").and_then(Value::as_object) {
        for child in children.values() {
            if let Some(child) = child.as_object() {
                let (cdl, cul) = collect_raw_node_commits(child, circuit_mins_by_node, raw_by_node);
                dl += cdl;
                ul += cul;
            }
        }
    }
    if let Some(node_id) = node_id {
        raw_by_node.insert(node_id.to_string(), (dl, ul));
    }
    (dl, ul)
}

/// Recursively writes each node's commit, capped by ceil and parent commit.
///
/// Mirrors `inheritBandwidthMaxes` + `ensure_min_less_than_max`: commit is
/// floored at the operator-provided `*BandwidthMbpsMin` (when present), capped
/// at `min(commit, ceil, parent_commit)`, then forced strictly below ceil.
fn write_node_commits(
    node: &mut Map<String, Value>,
    raw_by_node: &HashMap<String, (f64, f64)>,
    parent_commit_dl: f64,
    parent_commit_ul: f64,
) {
    let node_id = node.get("id").and_then(Value::as_str);
    let ceil_dl = read_node_rate_mbps(node, "downloadBandwidthMbps")
        .map(|v| v as f64)
        .unwrap_or(0.0);
    let ceil_ul = read_node_rate_mbps(node, "uploadBandwidthMbps")
        .map(|v| v as f64)
        .unwrap_or(0.0);
    let (raw_dl, raw_ul) = node_id
        .and_then(|id| raw_by_node.get(id).copied())
        .unwrap_or((0.0, 0.0));
    let existing_dl = read_node_rate_mbps(node, "downloadBandwidthMbpsMin")
        .map(|v| v as f64)
        .unwrap_or(0.0);
    let existing_ul = read_node_rate_mbps(node, "uploadBandwidthMbpsMin")
        .map(|v| v as f64)
        .unwrap_or(0.0);

    let commit_dl = clamp_node_commit(existing_dl.max(raw_dl), ceil_dl, parent_commit_dl);
    let commit_ul = clamp_node_commit(existing_ul.max(raw_ul), ceil_ul, parent_commit_ul);

    write_rate_min(node, "downloadBandwidthMbpsMin", commit_dl);
    write_rate_min(node, "uploadBandwidthMbpsMin", commit_ul);

    if let Some(children) = node.get_mut("children").and_then(Value::as_object_mut) {
        for child in children.values_mut() {
            if let Some(child) = child.as_object_mut() {
                write_node_commits(child, raw_by_node, commit_dl, commit_ul);
            }
        }
    }
}

/// Caps a node commit at its ceil and parent commit, then forces it strictly
/// below ceil (Linux HTB requires `rate < ceil` for site classes).
fn clamp_node_commit(commit: f64, ceil: f64, parent_commit: f64) -> f64 {
    let mut value = commit.min(ceil).min(parent_commit).max(0.0);
    if ceil > 0.0 && value >= ceil {
        value = if ceil >= 1.0 { ceil - 1.0 } else { 0.01 };
    }
    value
}

/// Writes a commit value as a whole-Mbps integer when possible, otherwise as a
/// two-decimal fraction (sub-Mbps guarantees).
fn write_rate_min(node: &mut Map<String, Value>, key: &str, commit: f64) {
    let number = if commit.fract().abs() < f64::EPSILON {
        serde_json::Number::from(commit as u64)
    } else {
        let rounded = (commit * 100.0).round() / 100.0;
        serde_json::Number::from_f64(rounded).unwrap_or_else(|| serde_json::Number::from(0))
    };
    node.insert(key.to_string(), Value::Number(number));
}
