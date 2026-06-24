fn parse_probe_ip(raw: &str) -> Option<IpAddr> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let Some((addr, prefix)) = raw.split_once('/') else {
        return raw.parse::<IpAddr>().ok();
    };
    if addr.is_empty() || prefix.is_empty() || prefix.contains('/') {
        return None;
    }
    let ip = addr.parse::<IpAddr>().ok()?;
    let prefix = prefix.parse::<u8>().ok()?;
    match ip {
        IpAddr::V4(_) if prefix <= 32 => Some(ip),
        IpAddr::V6(_) if prefix <= 128 => Some(ip),
        _ => None,
    }
}

/// Returns the runtime stale cutoff in seconds for topology attachment health.
pub fn health_state_stale_after_seconds(config: &Config) -> u64 {
    config
        .integration_common
        .topology_attachment_health
        .probe_interval_seconds
        .saturating_mul(
            u64::from(
                config
                    .integration_common
                    .topology_attachment_health
                    .fail_after_missed,
            )
            .saturating_mul(3),
        )
}

/// Returns true when `health` is recent enough to be trusted for runtime suppression.
pub fn is_health_state_fresh(config: &Config, health: &TopologyAttachmentHealthStateFile) -> bool {
    let Some(generated_unix) = health.generated_unix else {
        return false;
    };
    let Some(now) = now_unix() else {
        return false;
    };
    now.saturating_sub(generated_unix) <= health_state_stale_after_seconds(config)
}

fn auto_attachment_option() -> TopologyAttachmentOption {
    TopologyAttachmentOption {
        attachment_id: TOPOLOGY_ATTACHMENT_AUTO_ID.to_string(),
        attachment_name: "Auto".to_string(),
        attachment_kind: "auto".to_string(),
        attachment_role: TopologyAttachmentRole::Unknown,
        pair_id: None,
        peer_attachment_id: None,
        peer_attachment_name: None,
        capacity_mbps: None,
        download_bandwidth_mbps: None,
        upload_bandwidth_mbps: None,
        transport_cap_mbps: None,
        transport_cap_reason: None,
        rate_source: TopologyAttachmentRateSource::Unknown,
        can_override_rate: false,
        rate_override_disabled_reason: None,
        has_rate_override: false,
        local_probe_ip: None,
        remote_probe_ip: None,
        probe_enabled: false,
        probeable: false,
        health_status: TopologyAttachmentHealthStatus::Disabled,
        health_reason: None,
        suppressed_until_unix: None,
        effective_selected: false,
    }
}

fn overlay_manual_groups(
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
) -> TopologyEditorStateFile {
    let mut state = canonical.clone();
    for node in &mut state.nodes {
        for parent in &mut node.allowed_parents {
            let Some(group) =
                overrides.find_manual_attachment_group(&node.node_id, &parent.parent_node_id)
            else {
                continue;
            };
            let mut options = vec![auto_attachment_option()];
            for attachment in &group.attachments {
                let local_probe_ip = parse_probe_ip(&attachment.local_probe_ip);
                let remote_probe_ip = parse_probe_ip(&attachment.remote_probe_ip);
                let probeable = local_probe_ip
                    .zip(remote_probe_ip)
                    .is_some_and(|(local, remote)| local != remote);
                options.push(TopologyAttachmentOption {
                    attachment_id: attachment.attachment_id.clone(),
                    attachment_name: attachment.attachment_name.clone(),
                    attachment_kind: "manual".to_string(),
                    attachment_role: TopologyAttachmentRole::Manual,
                    pair_id: Some(attachment.attachment_id.clone()),
                    peer_attachment_id: None,
                    peer_attachment_name: None,
                    capacity_mbps: Some(attachment.capacity_mbps),
                    download_bandwidth_mbps: Some(attachment.capacity_mbps),
                    upload_bandwidth_mbps: Some(attachment.capacity_mbps),
                    transport_cap_mbps: None,
                    transport_cap_reason: None,
                    rate_source: TopologyAttachmentRateSource::Manual,
                    can_override_rate: true,
                    rate_override_disabled_reason: None,
                    has_rate_override: false,
                    local_probe_ip: Some(attachment.local_probe_ip.clone()),
                    remote_probe_ip: Some(attachment.remote_probe_ip.clone()),
                    probe_enabled: attachment.probe_enabled,
                    probeable,
                    health_status: if attachment.probe_enabled {
                        if probeable {
                            TopologyAttachmentHealthStatus::Healthy
                        } else {
                            TopologyAttachmentHealthStatus::ProbeUnavailable
                        }
                    } else {
                        TopologyAttachmentHealthStatus::Disabled
                    },
                    health_reason: None,
                    suppressed_until_unix: None,
                    effective_selected: false,
                });
            }
            parent.attachment_options = options;
        }
    }
    state
}

fn attachment_capacity_mbps(option: &TopologyAttachmentOption) -> Option<u64> {
    match (
        option.download_bandwidth_mbps,
        option.upload_bandwidth_mbps,
        option.capacity_mbps,
    ) {
        (Some(download), Some(upload), _) => Some(download.min(upload)),
        (Some(download), None, _) => Some(download),
        (None, Some(upload), _) => Some(upload),
        (None, None, capacity) => capacity,
    }
}

fn apply_attachment_rate_overrides(
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
) -> TopologyEditorStateFile {
    let mut state = canonical.clone();
    for node in &mut state.nodes {
        for parent in &mut node.allowed_parents {
            for option in &mut parent.attachment_options {
                if option.attachment_id == TOPOLOGY_ATTACHMENT_AUTO_ID {
                    continue;
                }
                let Some(rate_override) = overrides.find_attachment_rate_override(
                    &node.node_id,
                    &parent.parent_node_id,
                    &option.attachment_id,
                ) else {
                    continue;
                };
                if !option.can_override_rate {
                    continue;
                }

                option.download_bandwidth_mbps = Some(rate_override.download_bandwidth_mbps);
                option.upload_bandwidth_mbps = Some(rate_override.upload_bandwidth_mbps);
                option.capacity_mbps = attachment_capacity_mbps(option);
                option.has_rate_override = true;
            }
        }
    }
    state
}

fn probe_enabled_for_option(
    option: &TopologyAttachmentOption,
    overrides: &TopologyOverridesFile,
) -> bool {
    let Some(pair_id) = option.pair_id.as_ref() else {
        return false;
    };
    overrides
        .find_probe_policy(pair_id)
        .map(|policy| policy.enabled)
        .unwrap_or(option.probe_enabled)
}

fn probe_unavailable_reason(local_ip: Option<&str>, remote_ip: Option<&str>) -> String {
    let local = local_ip.map(str::trim).unwrap_or_default();
    let remote = remote_ip.map(str::trim).unwrap_or_default();

    if local.is_empty() && remote.is_empty() {
        return "Probe unavailable: missing local and remote management IPs".to_string();
    }
    if local.is_empty() {
        return "Probe unavailable: missing local management IP".to_string();
    }
    if remote.is_empty() {
        return "Probe unavailable: missing remote management IP".to_string();
    }
    let local_ip = parse_probe_ip(local);
    let remote_ip = parse_probe_ip(remote);
    if local_ip
        .zip(remote_ip)
        .is_some_and(|(local, remote)| local == remote)
    {
        return "Probe unavailable: local and remote probe IPs are identical".to_string();
    }
    if local_ip.is_none() && remote_ip.is_none() {
        return "Probe unavailable: local and remote probe IPs are invalid".to_string();
    }
    if local_ip.is_none() {
        return "Probe unavailable: local management IP is invalid".to_string();
    }
    if remote_ip.is_none() {
        return "Probe unavailable: remote management IP is invalid".to_string();
    }
    "Probe unavailable".to_string()
}

fn apply_health_to_option(
    option: &TopologyAttachmentOption,
    overrides: &TopologyOverridesFile,
    health_by_pair: &HashMap<&str, &lqos_config::TopologyAttachmentHealthEntry>,
) -> TopologyAttachmentOption {
    if option.attachment_id == TOPOLOGY_ATTACHMENT_AUTO_ID {
        return option.clone();
    }

    let enabled = probe_enabled_for_option(option, overrides);
    let probeable = option
        .local_probe_ip
        .as_ref()
        .zip(option.remote_probe_ip.as_ref())
        .and_then(|(local, remote)| parse_probe_ip(local).zip(parse_probe_ip(remote)))
        .is_some_and(|(local, remote)| local != remote);

    let (health_status, health_reason, suppressed_until_unix) = if !enabled {
        (
            TopologyAttachmentHealthStatus::Disabled,
            Some("Health probe disabled".to_string()),
            None,
        )
    } else if !probeable {
        (
            TopologyAttachmentHealthStatus::ProbeUnavailable,
            Some(probe_unavailable_reason(
                option.local_probe_ip.as_deref(),
                option.remote_probe_ip.as_deref(),
            )),
            None,
        )
    } else if let Some(pair_id) = option.pair_id.as_deref() {
        if let Some(entry) = health_by_pair.get(pair_id) {
            (
                entry.status,
                entry.reason.clone(),
                entry.suppressed_until_unix,
            )
        } else {
            (
                TopologyAttachmentHealthStatus::ProbeUnavailable,
                Some(format!(
                    "Probe unavailable: no current health observation for pair '{pair_id}'"
                )),
                None,
            )
        }
    } else {
        (
            TopologyAttachmentHealthStatus::ProbeUnavailable,
            Some("Probe unavailable: missing attachment pair id".to_string()),
            None,
        )
    };

    let mut out = option.clone();
    out.probe_enabled = enabled;
    out.probeable = probeable;
    out.health_status = health_status;
    out.health_reason = health_reason;
    out.suppressed_until_unix = suppressed_until_unix;
    out
}

fn enrich_allowed_parent(
    parent: &TopologyAllowedParent,
    overrides: &TopologyOverridesFile,
    health_by_pair: &HashMap<&str, &lqos_config::TopologyAttachmentHealthEntry>,
    effective_attachment_id: Option<&str>,
    effective_parent_id: Option<&str>,
) -> TopologyAllowedParent {
    let mut all_attachments_suppressed = true;
    let mut has_probe_unavailable = false;
    let mut saw_explicit = false;
    let attachment_options = parent
        .attachment_options
        .iter()
        .map(|option| {
            let mut option = apply_health_to_option(option, overrides, health_by_pair);
            if option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID {
                saw_explicit = true;
                if option.health_status != TopologyAttachmentHealthStatus::Suppressed {
                    all_attachments_suppressed = false;
                }
                if option.health_status == TopologyAttachmentHealthStatus::ProbeUnavailable {
                    has_probe_unavailable = true;
                }
            }
            option.effective_selected = effective_parent_id == Some(parent.parent_node_id.as_str())
                && effective_attachment_id == Some(option.attachment_id.as_str());
            option
        })
        .collect::<Vec<_>>();

    TopologyAllowedParent {
        parent_node_id: parent.parent_node_id.clone(),
        parent_node_name: parent.parent_node_name.clone(),
        attachment_options,
        all_attachments_suppressed: saw_explicit && all_attachments_suppressed,
        has_probe_unavailable_attachments: has_probe_unavailable,
    }
}

fn valid_attachment_ids(parent: &TopologyAllowedParent) -> HashSet<&str> {
    parent
        .attachment_options
        .iter()
        .filter(|option| option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID)
        .map(|option| option.attachment_id.as_str())
        .collect()
}

fn option_name(parent: &TopologyAllowedParent, attachment_id: &str) -> Option<String> {
    parent
        .attachment_options
        .iter()
        .find(|option| option.attachment_id == attachment_id)
        .map(|option| option.attachment_name.clone())
}

fn parent_has_attachment(parent: &TopologyAllowedParent, attachment_id: &str) -> bool {
    parent
        .attachment_options
        .iter()
        .any(|option| option.attachment_id == attachment_id)
}

fn attachment_selectable_for_auto(option: &TopologyAttachmentOption) -> bool {
    option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID
        && option.health_status != TopologyAttachmentHealthStatus::Suppressed
}

const fn attachment_rate_source_preference(source: TopologyAttachmentRateSource) -> u8 {
    match source {
        TopologyAttachmentRateSource::DynamicIntegration => 3,
        TopologyAttachmentRateSource::Manual => 2,
        TopologyAttachmentRateSource::Static => 1,
        TopologyAttachmentRateSource::Unknown => 0,
    }
}

const fn attachment_health_preference(status: TopologyAttachmentHealthStatus) -> u8 {
    match status {
        TopologyAttachmentHealthStatus::Healthy => 3,
        TopologyAttachmentHealthStatus::Disabled => 2,
        TopologyAttachmentHealthStatus::ProbeUnavailable => 1,
        TopologyAttachmentHealthStatus::Suppressed => 0,
    }
}

const fn attachment_probeability_preference(option: &TopologyAttachmentOption) -> bool {
    match option.health_status {
        TopologyAttachmentHealthStatus::Healthy => option.probeable,
        TopologyAttachmentHealthStatus::Disabled => true,
        TopologyAttachmentHealthStatus::ProbeUnavailable
        | TopologyAttachmentHealthStatus::Suppressed => false,
    }
}

fn ranked_auto_attachment_id(
    parent: &TopologyAllowedParent,
    current_attachment_id: Option<&str>,
) -> Option<String> {
    parent
        .attachment_options
        .iter()
        .filter(|option| attachment_selectable_for_auto(option))
        .max_by_key(|option| {
            (
                attachment_health_preference(option.health_status),
                attachment_probeability_preference(option),
                attachment_rate_source_preference(option.rate_source),
                attachment_capacity_mbps(option).unwrap_or(0),
                current_attachment_id == Some(option.attachment_id.as_str()),
            )
        })
        .map(|option| option.attachment_id.clone())
}

fn first_selectable_attachment_id(parent: &TopologyAllowedParent) -> Option<String> {
    parent
        .attachment_options
        .iter()
        .find(|option| attachment_selectable_for_auto(option))
        .map(|option| option.attachment_id.clone())
}

fn first_explicit_attachment_id(parent: &TopologyAllowedParent) -> Option<String> {
    parent
        .attachment_options
        .iter()
        .find(|option| option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID)
        .map(|option| option.attachment_id.clone())
}

fn current_parent_for_node<'a>(
    node: &'a TopologyEditorNode,
    parent_id: &str,
) -> Option<&'a TopologyAllowedParent> {
    node.allowed_parents
        .iter()
        .find(|parent| parent.parent_node_id == parent_id)
}

fn runtime_may_infer_parent_from_candidates(source: &str) -> bool {
    !(source.starts_with("uisp/") || source.starts_with("python/"))
}

fn merge_attachment_option(
    existing: &mut TopologyAttachmentOption,
    incoming: &TopologyAttachmentOption,
) {
    if existing.attachment_name.is_empty() && !incoming.attachment_name.is_empty() {
        existing.attachment_name = incoming.attachment_name.clone();
    }
    if existing.attachment_kind.is_empty() && !incoming.attachment_kind.is_empty() {
        existing.attachment_kind = incoming.attachment_kind.clone();
    }
    if existing.attachment_role == TopologyAttachmentRole::Unknown
        && incoming.attachment_role != TopologyAttachmentRole::Unknown
    {
        existing.attachment_role = incoming.attachment_role;
    }
    if existing.pair_id.is_none() {
        existing.pair_id = incoming.pair_id.clone();
    }
    if existing.peer_attachment_id.is_none() {
        existing.peer_attachment_id = incoming.peer_attachment_id.clone();
    }
    if existing.peer_attachment_name.is_none() {
        existing.peer_attachment_name = incoming.peer_attachment_name.clone();
    }
    if existing.capacity_mbps.is_none() {
        existing.capacity_mbps = incoming.capacity_mbps;
    }
    if existing.download_bandwidth_mbps.is_none() {
        existing.download_bandwidth_mbps = incoming.download_bandwidth_mbps;
    }
    if existing.upload_bandwidth_mbps.is_none() {
        existing.upload_bandwidth_mbps = incoming.upload_bandwidth_mbps;
    }
    if existing.transport_cap_mbps.is_none() {
        existing.transport_cap_mbps = incoming.transport_cap_mbps;
    }
    if existing.transport_cap_reason.is_none() {
        existing.transport_cap_reason = incoming.transport_cap_reason.clone();
    }
    if existing.rate_source == TopologyAttachmentRateSource::Unknown
        && incoming.rate_source != TopologyAttachmentRateSource::Unknown
    {
        existing.rate_source = incoming.rate_source;
    }
    existing.can_override_rate |= incoming.can_override_rate;
    if existing.rate_override_disabled_reason.is_none() {
        existing.rate_override_disabled_reason = incoming.rate_override_disabled_reason.clone();
    }
    existing.has_rate_override |= incoming.has_rate_override;
    if existing.local_probe_ip.is_none() {
        existing.local_probe_ip = incoming.local_probe_ip.clone();
    }
    if existing.remote_probe_ip.is_none() {
        existing.remote_probe_ip = incoming.remote_probe_ip.clone();
    }
    existing.probe_enabled |= incoming.probe_enabled;
    existing.probeable |= incoming.probeable;
    if existing.health_status == TopologyAttachmentHealthStatus::Healthy
        && incoming.health_status != TopologyAttachmentHealthStatus::Healthy
    {
        existing.health_status = incoming.health_status;
    }
    if existing.health_reason.is_none() {
        existing.health_reason = incoming.health_reason.clone();
    }
    if existing.suppressed_until_unix.is_none() {
        existing.suppressed_until_unix = incoming.suppressed_until_unix;
    }
    existing.effective_selected |= incoming.effective_selected;
}

fn merge_allowed_parent(existing: &mut TopologyAllowedParent, incoming: &TopologyAllowedParent) {
    if existing.parent_node_name.is_empty() && !incoming.parent_node_name.is_empty() {
        existing.parent_node_name = incoming.parent_node_name.clone();
    }
    for option in &incoming.attachment_options {
        if let Some(existing_option) = existing
            .attachment_options
            .iter_mut()
            .find(|current| current.attachment_id == option.attachment_id)
        {
            merge_attachment_option(existing_option, option);
        } else {
            existing.attachment_options.push(option.clone());
        }
    }
    existing.all_attachments_suppressed &= incoming.all_attachments_suppressed;
    existing.has_probe_unavailable_attachments |= incoming.has_probe_unavailable_attachments;
}

fn normalize_topology_editor_state(canonical: &TopologyEditorStateFile) -> TopologyEditorStateFile {
    let mut nodes = Vec::<TopologyEditorNode>::new();
    let mut index_by_id = HashMap::<String, usize>::new();

    for node in &canonical.nodes {
        if let Some(index) = index_by_id.get(&node.node_id).copied() {
            let existing = &mut nodes[index];
            if existing.node_name.is_empty() && !node.node_name.is_empty() {
                existing.node_name = node.node_name.clone();
            }
            if existing.current_parent_node_id.is_none() {
                existing.current_parent_node_id = node.current_parent_node_id.clone();
            }
            if existing.current_parent_node_name.is_none() {
                existing.current_parent_node_name = node.current_parent_node_name.clone();
            }
            if existing.current_attachment_id.is_none() {
                existing.current_attachment_id = node.current_attachment_id.clone();
            }
            if existing.current_attachment_name.is_none() {
                existing.current_attachment_name = node.current_attachment_name.clone();
            }
            existing.can_move |= node.can_move;
            for parent in &node.allowed_parents {
                if let Some(existing_parent) = existing
                    .allowed_parents
                    .iter_mut()
                    .find(|current| current.parent_node_id == parent.parent_node_id)
                {
                    merge_allowed_parent(existing_parent, parent);
                } else {
                    existing.allowed_parents.push(parent.clone());
                }
            }
            if existing.preferred_attachment_id.is_none() {
                existing.preferred_attachment_id = node.preferred_attachment_id.clone();
            }
            if existing.preferred_attachment_name.is_none() {
                existing.preferred_attachment_name = node.preferred_attachment_name.clone();
            }
            if existing.effective_attachment_id.is_none() {
                existing.effective_attachment_id = node.effective_attachment_id.clone();
            }
            if existing.effective_attachment_name.is_none() {
                existing.effective_attachment_name = node.effective_attachment_name.clone();
            }
            continue;
        }

        index_by_id.insert(node.node_id.clone(), nodes.len());
        nodes.push(node.clone());
    }

    for node in &mut nodes {
        if let Some(current_parent_id) = node.current_parent_node_id.as_deref()
            && node
                .allowed_parents
                .iter()
                .all(|parent| parent.parent_node_id != current_parent_id)
            && let Some(fallback_parent) = node.allowed_parents.first()
        {
            node.current_parent_node_id = Some(fallback_parent.parent_node_id.clone());
            node.current_parent_node_name = Some(fallback_parent.parent_node_name.clone());
        }
    }

    TopologyEditorStateFile {
        schema_version: canonical.schema_version,
        source: canonical.source.clone(),
        generated_unix: canonical.generated_unix,
        ingress_identity: canonical.ingress_identity.clone(),
        nodes,
    }
}

fn prepared_runtime_topology_editor_state(
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
) -> TopologyEditorStateFile {
    let normalized = normalize_topology_editor_state(canonical);
    let manual = overlay_manual_groups(&normalized, overrides);
    apply_attachment_rate_overrides(&manual, overrides)
}

fn health_entries_by_pair<'a>(
    config: &Config,
    health: &'a TopologyAttachmentHealthStateFile,
) -> HashMap<&'a str, &'a lqos_config::TopologyAttachmentHealthEntry> {
    if is_health_state_fresh(config, health) {
        health
            .attachments
            .iter()
            .map(|entry| (entry.attachment_pair_id.as_str(), entry))
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    }
}

/// Prepares the runtime editor-state view used for probe planning and effective compilation.
///
/// Side effects: none. This applies normalization, manual attachment overlays, and attachment
/// rate overrides to the canonical topology editor state.
pub fn prepare_runtime_topology_editor_state_from_canonical(
    canonical: &TopologyCanonicalStateFile,
    overrides: &TopologyOverridesFile,
) -> TopologyEditorStateFile {
    prepared_runtime_topology_editor_state(&canonical.to_editor_state(), overrides)
}

/// Computes the effective attachment selection for all nodes using canonical state,
/// operator intent, and transient runtime health.
pub fn compute_effective_state(
    config: &Config,
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> TopologyEffectiveStateFile {
    let prepared = prepared_runtime_topology_editor_state(canonical, overrides);
    compute_effective_state_from_prepared(config, &prepared, overrides, health)
}

fn compute_effective_state_from_prepared(
    config: &Config,
    prepared: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> TopologyEffectiveStateFile {
    let health_by_pair = health_entries_by_pair(config, health);
    let may_infer_parent = runtime_may_infer_parent_from_candidates(&prepared.source);

    let mut nodes = Vec::with_capacity(prepared.nodes.len());
    for node in &prepared.nodes {
        let selected_parent_id = overrides
            .find_override(&node.node_id)
            .and_then(|saved| {
                current_parent_for_node(node, &saved.parent_node_id)
                    .map(|parent| parent.parent_node_id.clone())
            })
            .or_else(|| node.current_parent_node_id.clone())
            .or_else(|| {
                may_infer_parent
                    .then(|| {
                        node.allowed_parents
                            .first()
                            .map(|parent| parent.parent_node_id.clone())
                    })
                    .flatten()
            });

        let Some(selected_parent_id) = selected_parent_id else {
            nodes.push(TopologyEffectiveNodeState {
                node_id: node.node_id.clone(),
                logical_parent_node_id: String::new(),
                preferred_attachment_id: None,
                effective_attachment_id: None,
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: Vec::new(),
            });
            continue;
        };

        let Some(selected_parent) =
            current_parent_for_node(node, &selected_parent_id).or_else(|| {
                may_infer_parent
                    .then(|| node.allowed_parents.first())
                    .flatten()
            })
        else {
            let fixed_attachment_id = node
                .current_attachment_id
                .clone()
                .filter(|attachment_id| !attachment_id.is_empty());
            nodes.push(TopologyEffectiveNodeState {
                node_id: node.node_id.clone(),
                logical_parent_node_id: selected_parent_id,
                preferred_attachment_id: fixed_attachment_id.clone(),
                effective_attachment_id: fixed_attachment_id,
                fallback_reason: None,
                all_attachments_suppressed: false,
                attachments: Vec::new(),
            });
            continue;
        };
        let selected_parent_id = selected_parent.parent_node_id.clone();
        let enriched_parent = enrich_allowed_parent(
            selected_parent,
            overrides,
            &health_by_pair,
            None,
            Some(&selected_parent_id),
        );

        let explicit_options = enriched_parent
            .attachment_options
            .iter()
            .filter(|option| option.attachment_id != TOPOLOGY_ATTACHMENT_AUTO_ID)
            .cloned()
            .collect::<Vec<_>>();

        let override_entry = overrides.find_override(&node.node_id);
        let preferred_attachment_id = match override_entry {
            Some(saved)
                if saved.parent_node_id == selected_parent_id
                    && saved.mode == TopologyAttachmentMode::PreferredOrder =>
            {
                let valid_ids = valid_attachment_ids(&enriched_parent);
                saved
                    .attachment_preference_ids
                    .iter()
                    .find(|attachment_id| valid_ids.contains(attachment_id.as_str()))
                    .cloned()
                    .or_else(|| {
                        node.current_attachment_id.clone().filter(|attachment_id| {
                            parent_has_attachment(&enriched_parent, attachment_id)
                        })
                    })
            }
            _ => ranked_auto_attachment_id(&enriched_parent, node.current_attachment_id.as_deref())
                .or_else(|| {
                    node.current_attachment_id.clone().filter(|attachment_id| {
                        parent_has_attachment(&enriched_parent, attachment_id)
                    })
                }),
        };

        let selectable_ids = explicit_options
            .iter()
            .filter(|option| attachment_selectable_for_auto(option))
            .map(|option| option.attachment_id.clone())
            .collect::<HashSet<_>>();

        let mut fallback_reason = None;
        let effective_attachment_id = if explicit_options.is_empty() {
            None
        } else if !selectable_ids.is_empty() {
            match override_entry {
                Some(saved)
                    if saved.parent_node_id == selected_parent_id
                        && saved.mode == TopologyAttachmentMode::PreferredOrder =>
                {
                    saved
                        .attachment_preference_ids
                        .iter()
                        .find(|attachment_id| selectable_ids.contains(*attachment_id))
                        .cloned()
                        .or_else(|| {
                            node.current_attachment_id
                                .clone()
                                .filter(|attachment_id| selectable_ids.contains(attachment_id))
                        })
                        .or_else(|| {
                            ranked_auto_attachment_id(
                                &enriched_parent,
                                node.current_attachment_id.as_deref(),
                            )
                        })
                        .or_else(|| first_selectable_attachment_id(&enriched_parent))
                }
                _ => ranked_auto_attachment_id(
                    &enriched_parent,
                    node.current_attachment_id.as_deref(),
                )
                .or_else(|| first_selectable_attachment_id(&enriched_parent)),
            }
        } else {
            fallback_reason = Some(if enriched_parent.all_attachments_suppressed {
                "All attachments suppressed; using deterministic fallback".to_string()
            } else {
                "No healthy attachment available; using deterministic fallback".to_string()
            });
            node.current_attachment_id
                .clone()
                .filter(|attachment_id| parent_has_attachment(&enriched_parent, attachment_id))
                .or_else(|| first_explicit_attachment_id(&enriched_parent))
        };

        let attachments = explicit_options
            .iter()
            .map(|option| TopologyEffectiveAttachmentState {
                attachment_id: option.attachment_id.clone(),
                health_status: option.health_status,
                health_reason: option.health_reason.clone(),
                suppressed_until_unix: option.suppressed_until_unix,
                probe_enabled: option.probe_enabled,
                probeable: option.probeable,
                effective_selected: effective_attachment_id
                    .as_deref()
                    .is_some_and(|id| id == option.attachment_id),
            })
            .collect::<Vec<_>>();

        nodes.push(TopologyEffectiveNodeState {
            node_id: node.node_id.clone(),
            logical_parent_node_id: selected_parent_id.clone(),
            preferred_attachment_id,
            effective_attachment_id,
            fallback_reason,
            all_attachments_suppressed: enriched_parent.all_attachments_suppressed,
            attachments,
        });
    }

    TopologyEffectiveStateFile {
        schema_version: 1,
        generated_unix: now_unix(),
        canonical_generated_unix: prepared.generated_unix,
        health_generated_unix: health.generated_unix,
        nodes,
    }
}

/// Builds a UI-facing topology editor state with manual groups, probe enablement,
/// health annotations, and effective attachment selection applied.
pub fn merged_topology_state(
    config: &Config,
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
    effective: &TopologyEffectiveStateFile,
) -> TopologyEditorStateFile {
    let prepared = prepared_runtime_topology_editor_state(canonical, overrides);
    merged_topology_state_from_prepared(config, &prepared, overrides, health, effective)
}

fn merged_topology_state_from_prepared(
    config: &Config,
    prepared: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
    effective: &TopologyEffectiveStateFile,
) -> TopologyEditorStateFile {
    let mut state = prepared.clone();
    let health_by_pair = health_entries_by_pair(config, health);
    let effective_by_node = effective
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();

    for node in &mut state.nodes {
        let effective_node = effective_by_node.get(node.node_id.as_str()).copied();
        let effective_parent_id = effective_node.map(|entry| entry.logical_parent_node_id.as_str());
        let effective_attachment_id =
            effective_node.and_then(|entry| entry.effective_attachment_id.as_deref());
        let preferred_attachment_id =
            effective_node.and_then(|entry| entry.preferred_attachment_id.as_deref());

        node.allowed_parents = node
            .allowed_parents
            .iter()
            .map(|parent| {
                enrich_allowed_parent(
                    parent,
                    overrides,
                    &health_by_pair,
                    effective_attachment_id,
                    effective_parent_id,
                )
            })
            .collect();
        node.preferred_attachment_id = preferred_attachment_id.map(ToString::to_string);
        node.preferred_attachment_name = preferred_attachment_id.and_then(|attachment_id| {
            node.allowed_parents
                .iter()
                .find_map(|parent| option_name(parent, attachment_id))
        });
        node.effective_attachment_id = effective_attachment_id.map(ToString::to_string);
        node.effective_attachment_name = effective_attachment_id.and_then(|attachment_id| {
            node.allowed_parents
                .iter()
                .find_map(|parent| option_name(parent, attachment_id))
        });
    }

    state
}

/// Emits the unique set of enabled/known probe specs from the UI-facing topology state.
pub fn probe_specs_from_state(
    state: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
) -> Vec<AttachmentProbeSpec> {
    let mut seen = HashSet::new();
    let mut specs = Vec::new();
    for node in &state.nodes {
        for parent in &node.allowed_parents {
            for option in &parent.attachment_options {
                if option.attachment_id == TOPOLOGY_ATTACHMENT_AUTO_ID {
                    continue;
                }
                let Some(pair_id) = option.pair_id.clone() else {
                    continue;
                };
                let Some(local_ip) = option.local_probe_ip.clone() else {
                    continue;
                };
                let Some(remote_ip) = option.remote_probe_ip.clone() else {
                    continue;
                };
                if !seen.insert(pair_id.clone()) {
                    continue;
                }
                specs.push(AttachmentProbeSpec {
                    pair_id: pair_id.clone(),
                    attachment_id: option.attachment_id.clone(),
                    attachment_name: option.attachment_name.clone(),
                    node_id: node.node_id.clone(),
                    node_name: node.node_name.clone(),
                    parent_node_id: parent.parent_node_id.clone(),
                    parent_node_name: parent.parent_node_name.clone(),
                    local_ip,
                    remote_ip,
                    enabled: probe_enabled_for_option(option, overrides),
                });
            }
        }
    }
    specs.sort_unstable_by(|left, right| left.pair_id.cmp(&right.pair_id));
    specs
}
