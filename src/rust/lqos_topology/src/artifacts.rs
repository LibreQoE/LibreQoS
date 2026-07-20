/// Loads canonical topology state, falling back to importing legacy `network.json`.
pub fn load_canonical_topology_state(config: &Config) -> TopologyCanonicalStateFile {
    TopologyCanonicalStateFile::load_with_legacy_fallback(config).unwrap_or_default()
}

/// Validated effective-topology artifacts ready for publication.
#[derive(Clone, Debug)]
pub struct EffectiveTopologyArtifacts {
    /// Runtime-effective topology state derived from canonical topology and overrides.
    pub effective: TopologyEffectiveStateFile,
    /// UI-facing merged topology state derived from canonical topology, overrides, and health.
    pub ui_state: TopologyEditorStateFile,
    /// Runtime-effective tree used by shaping/export when canonical compatibility network exists.
    pub effective_network: Option<Value>,
}

/// Builds validated effective-topology artifacts from canonical topology state.
pub fn build_effective_topology_artifacts_from_canonical(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> std::result::Result<EffectiveTopologyArtifacts, Vec<String>> {
    let prepared = prepared_runtime_topology_editor_state(&canonical.to_editor_state(), overrides);
    let virtualization = QueueVirtualizationContext::default();
    build_effective_topology_artifacts_from_prepared(
        config,
        canonical,
        &prepared,
        overrides,
        health,
        &virtualization,
    )
}

/// Builds validated effective-topology artifacts with runtime queue-virtualization guards.
///
/// Side effects: reads shaping artifacts and effective override layers so automatic
/// virtualization can avoid nodes that have direct circuit attachments or explicit
/// visible overrides.
pub fn build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> std::result::Result<EffectiveTopologyArtifacts, Vec<String>> {
    let prepared = prepared_runtime_topology_editor_state(&canonical.to_editor_state(), overrides);
    let virtualization = load_queue_virtualization_context(config, &prepared).map_err(|err| {
        vec![format!(
            "Unable to load queue virtualization context: {err:#}"
        )]
    })?;
    build_effective_topology_artifacts_from_prepared(
        config,
        canonical,
        &prepared,
        overrides,
        health,
        &virtualization,
    )
}

/// Builds validated effective-topology artifacts from legacy editor state plus compatibility
/// `network.json`.
///
/// This helper preserves existing test call sites while routing through the canonical topology
/// model used in production.
pub fn build_effective_topology_artifacts(
    config: &Config,
    canonical: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
    canonical_network: Option<&Value>,
) -> std::result::Result<EffectiveTopologyArtifacts, Vec<String>> {
    let canonical_state = TopologyCanonicalStateFile::from_editor_and_network(
        canonical,
        canonical_network.unwrap_or(&Value::Object(Map::new())),
        lqos_config::TopologyCanonicalIngressKind::NativeIntegration,
    );
    build_effective_topology_artifacts_from_canonical(config, &canonical_state, overrides, health)
}

fn build_effective_topology_artifacts_from_prepared(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    prepared: &TopologyEditorStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
    virtualization: &QueueVirtualizationContext,
) -> std::result::Result<EffectiveTopologyArtifacts, Vec<String>> {
    let effective = compute_effective_state_from_prepared(config, prepared, overrides, health);
    let ui_state =
        merged_topology_state_from_prepared(config, prepared, overrides, health, &effective);
    let canonical_network =
        if canonical.ingress_kind == TopologyCanonicalIngressKind::NativeIntegration {
            canonical.insight_topology_network_json()
        } else {
            canonical.compatibility_network_json().clone()
        };
    let effective_network = if runtime_flat_mode(config) {
        Some(runtime_flat_bucket_network(config))
    } else {
        match canonical_network.as_object() {
            Some(_) => Some(apply_effective_topology_to_canonical_state(
                config,
                canonical,
                &ui_state,
                &effective,
                virtualization,
            )?),
            None => None,
        }
    };

    if let Some(effective_network) = effective_network.as_ref() {
        validate_effective_topology_network_from_canonical(
            config,
            canonical,
            &ui_state,
            &effective,
            effective_network,
            virtualization,
        )?;
    }

    Ok(EffectiveTopologyArtifacts {
        effective,
        ui_state,
        effective_network,
    })
}
