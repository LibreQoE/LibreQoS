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
    /// T5 rung 1: the canonical model the artifacts were built from —
    /// E2's node source (full operator topology, including circuit-less
    /// sites the shaping projection elides).
    pub canonical: TopologyCanonicalStateFile,
}

/// Builds validated effective-topology artifacts from canonical topology state
/// using the DEFAULT (empty) direct-circuit-host set.
///
/// Test-only by design (1.1 dual-builder fix, 2026-09-10): production MUST use
/// [`build_effective_topology_artifacts_from_canonical_with_runtime_queue_context`],
/// which loads the real direct-circuit-host set. This variant is retained
/// only for unit tests/legacy fixtures; it produces a DIFFERENT effective
/// network (and therefore a different shaping generation) whenever direct
/// circuit hosts affect runtime squashing, so any production caller would
/// publish a divergent world. Keeping it `#[cfg(test)]` makes a production
/// caller a compile error.
#[cfg(test)]
pub(crate) fn build_effective_topology_artifacts_from_canonical(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> std::result::Result<EffectiveTopologyArtifacts, Vec<String>> {
    let prepared = prepared_runtime_topology_editor_state(&canonical.to_editor_state(), overrides);
    let direct_circuit_hosts = DirectCircuitHosts::default();
    build_effective_topology_artifacts_from_prepared(
        config,
        canonical,
        &prepared,
        overrides,
        health,
        &direct_circuit_hosts,
        false,
    )
}

/// Builds validated effective-topology artifacts with runtime direct-circuit-host guards.
///
/// Side effects: reads shaping artifacts and effective override layers so runtime
/// squashing can avoid nodes that have direct circuit attachments.
pub fn build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> std::result::Result<EffectiveTopologyArtifacts, Vec<String>> {
    build_effective_topology_artifacts_from_canonical_with_runtime_queue_context_with(
        config, canonical, overrides, health, false,
    )
}

/// Read-only variant of
/// [`build_effective_topology_artifacts_from_canonical_with_runtime_queue_context`]:
/// loads the effective override layers WITHOUT creating or migrating the
/// operator overrides file. A read path must never write (the scheduler
/// readiness verifier runs this per tick).
pub fn build_effective_topology_artifacts_from_canonical_with_runtime_queue_context_read_only(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
) -> std::result::Result<EffectiveTopologyArtifacts, Vec<String>> {
    build_effective_topology_artifacts_from_canonical_with_runtime_queue_context_with(
        config, canonical, overrides, health, true,
    )
}

fn build_effective_topology_artifacts_from_canonical_with_runtime_queue_context_with(
    config: &Config,
    canonical: &TopologyCanonicalStateFile,
    overrides: &TopologyOverridesFile,
    health: &TopologyAttachmentHealthStateFile,
    read_only: bool,
) -> std::result::Result<EffectiveTopologyArtifacts, Vec<String>> {
    let prepared = prepared_runtime_topology_editor_state(&canonical.to_editor_state(), overrides);
    let direct_circuit_hosts =
        load_direct_circuit_hosts(config, &prepared, read_only).map_err(|err| {
            vec![format!(
                "Unable to load direct-circuit-host set: {err:#}"
            )]
        })?;
    build_effective_topology_artifacts_from_prepared(
        config,
        canonical,
        &prepared,
        overrides,
        health,
        &direct_circuit_hosts,
        read_only,
    )
}

/// Builds validated effective-topology artifacts from legacy editor state plus compatibility
/// `network.json`.
///
/// Test-only by design (1.1 dual-builder fix, 2026-09-10): this legacy helper
/// routes through the DEFAULT direct-circuit-host set and is retained only
/// for unit tests; production must use the runtime-context builder.
#[cfg(test)]
pub(crate) fn build_effective_topology_artifacts(
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
    direct_circuit_hosts: &DirectCircuitHosts,
    read_only: bool,
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
                direct_circuit_hosts,
                read_only,
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
        )?;
    }

    Ok(EffectiveTopologyArtifacts {
        effective,
        ui_state,
        effective_network,
        canonical: canonical.clone(),
    })
}

/// Phase C S4 (producer unification): materializes the compiled ingest
/// artifacts from the PERMANENT manual pair (`network.json` +
/// `ShapedDevices.csv`) through the legacy door when an integration did not
/// produce them. Every consumer then reads `topology_import.json` /
/// `topology_compiled_shaping.json` only — the integration-ingress "mode" no
/// longer reaches a consumer.
///
/// Validation happens here, on INGEST: the legacy door's loaders reject
/// malformed rows and the compiler's `validate_compiled_bundle` rejects a
/// broken world, so a bad pair fails LOUD before the bridge sees it.
///
/// Side effects: reads the manual pair and writes `topology_import.json` plus
/// `topology_compiled_shaping.json`.
pub fn materialize_manual_topology_import(config: &Config) -> Result<bool> {
    // An integration (or a prior manual pass) already owns the import; never
    // overwrite it from the pair. The config-change refresh is the scheduler's
    // explicit producer call (`write_compiled_topology_from_legacy_artifacts`).
    if lqos_config::topology_import_path(config).exists()
        || lqos_config::topology_compiled_shaping_path(config).exists()
    {
        return Ok(false);
    }
    lqos_topology_compile::materialize_manual_topology(config)
        .context("Unable to materialize the manual pair topology import")
}
