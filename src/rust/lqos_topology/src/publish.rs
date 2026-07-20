struct EffectivePublishLock {
    _lock: ProcessFileLock,
}

fn acquire_effective_publish_lock(config: &Config) -> Result<EffectivePublishLock> {
    let lock_dir = Path::new(&config.lqos_directory);
    let lock_path = lock_dir.join(TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_FILENAME);
    let guard_path = lock_dir.join(TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_GUARD_FILENAME);
    let lock_config = ProcessLockConfig::new(
        &lock_path,
        lock_dir,
        guard_path,
        "publish effective topology artifacts",
        TOPOLOGY_EFFECTIVE_PUBLISH_LOCK_CONTENTION_CODE,
        "The LibreQoS topology effective publish lock",
    );
    let lock = ProcessFileLock::acquire(&lock_config).with_context(|| {
        format!(
            "Unable to acquire topology effective publish lock at {:?}",
            lock_path
        )
    })?;
    Ok(EffectivePublishLock { _lock: lock })
}

/// Publishes effective topology artifacts and shaping inputs for one source generation.
pub fn publish_effective_topology_artifacts(
    config: &Config,
    artifacts: &EffectiveTopologyArtifacts,
    source_generation: &str,
) -> Result<()> {
    let _lock = acquire_effective_publish_lock(config)?;

    let effective_state_path = topology_effective_state_path(config);
    let effective_state_value = serde_json::to_value(&artifacts.effective)?;

    let effective_network_path = topology_effective_network_path(config);

    let shaping_inputs_path = topology_shaping_inputs_path(config);
    let shaping_inputs = build_shaping_inputs(config, artifacts)?;
    let prepared_shaping_inputs = match shaping_inputs {
        Some(mut shaping_inputs) => {
            shaping_inputs.shaping_generation = shaping_inputs.compute_shaping_generation()?;
            let shaping_inputs_value = serde_json::to_value(&shaping_inputs)?;
            Some((shaping_inputs, shaping_inputs_value))
        }
        None => None,
    };

    let current_effective_state = TopologyEffectiveStateFile::load(config).ok();
    if !current_effective_state
        .as_ref()
        .is_some_and(|current| effective_state_payload_equals(current, &artifacts.effective))
    {
        atomic_write_json_value(&effective_state_path, &effective_state_value).with_context(
            || {
                format!(
                    "Unable to publish effective topology state at {:?}",
                    effective_state_path
                )
            },
        )?;
    }

    if let Some(effective_network) = artifacts.effective_network.as_ref() {
        let current_effective_network = read_json_value(&effective_network_path);
        if current_effective_network.as_ref() != Some(effective_network) {
            atomic_write_json_value(&effective_network_path, effective_network).with_context(
                || {
                    format!(
                        "Unable to publish effective topology network at {:?}",
                        effective_network_path
                    )
                },
            )?;
        }
    } else if effective_network_path.exists() {
        std::fs::remove_file(&effective_network_path).with_context(|| {
            format!(
                "Unable to remove stale effective topology network at {:?}",
                effective_network_path
            )
        })?;
    }

    let effective_generation = if artifacts.effective_network.is_some() {
        Some(
            compute_effective_network_file_generation(&effective_network_path).with_context(
                || {
                    format!(
                        "Unable to compute effective topology generation from {:?}",
                        effective_network_path
                    )
                },
            )?,
        )
    } else {
        None
    };

    match prepared_shaping_inputs {
        Some((shaping_inputs, shaping_inputs_value)) => {
            let current_shaping_inputs = TopologyShapingInputsFile::load(config).ok();
            if !current_shaping_inputs
                .as_ref()
                .is_some_and(|current| current.semantic_equals(&shaping_inputs))
            {
                atomic_write_json_value(&shaping_inputs_path, &shaping_inputs_value).with_context(
                    || {
                        format!(
                            "Unable to publish runtime shaping inputs at {:?}",
                            shaping_inputs_path
                        )
                    },
                )?;
            }
            publish_topology_runtime_status(
                config,
                source_generation,
                Some(&shaping_inputs.shaping_generation),
                effective_generation.as_deref(),
                true,
                None,
            )?;
        }
        None => {
            if shaping_inputs_path.exists() {
                std::fs::remove_file(&shaping_inputs_path).with_context(|| {
                    format!(
                        "Unable to remove stale runtime shaping inputs at {:?}",
                        shaping_inputs_path
                    )
                })?;
            }
            publish_topology_runtime_status(
                config,
                source_generation,
                None,
                effective_generation.as_deref(),
                false,
                Some("Topology runtime did not produce shaping inputs.".to_string()),
            )?;
        }
    }

    Ok(())
}

fn topology_runtime_status_snapshot(
    config: &Config,
    source_generation: &str,
    shaping_generation: Option<&str>,
    effective_generation: Option<&str>,
    ready: bool,
    error: Option<String>,
) -> TopologyRuntimeStatusFile {
    TopologyRuntimeStatusFile {
        schema_version: 1,
        source_generation: source_generation.to_string(),
        shaping_generation: shaping_generation.unwrap_or_default().to_string(),
        effective_generation: effective_generation.unwrap_or_default().to_string(),
        ready,
        generated_unix: now_unix(),
        effective_state_path: topology_effective_state_path(config)
            .to_string_lossy()
            .to_string(),
        effective_network_path: topology_effective_network_path(config)
            .to_string_lossy()
            .to_string(),
        shaping_inputs_path: topology_shaping_inputs_path(config)
            .to_string_lossy()
            .to_string(),
        error,
    }
}

/// Publishes topology runtime readiness for one source generation.
///
/// Side effects: writes `topology_runtime_status.json` in `config.lqos_directory`.
pub fn publish_topology_runtime_status(
    config: &Config,
    source_generation: &str,
    shaping_generation: Option<&str>,
    effective_generation: Option<&str>,
    ready: bool,
    error: Option<String>,
) -> Result<()> {
    let status = topology_runtime_status_snapshot(
        config,
        source_generation,
        shaping_generation,
        effective_generation,
        ready,
        error,
    );
    if TopologyRuntimeStatusFile::load(config)
        .ok()
        .as_ref()
        .is_some_and(|current| current.semantic_equals_for_publish(&status))
    {
        return Ok(());
    }
    status.save(config).with_context(|| {
        format!(
            "Unable to publish topology runtime status at {:?}",
            topology_runtime_status_path(config)
        )
    })?;
    Ok(())
}

/// Publishes a failed topology runtime status for one source generation.
///
/// Side effects: writes `topology_runtime_status.json` in `config.lqos_directory`.
pub fn publish_topology_runtime_error_status(
    config: &Config,
    source_generation: &str,
    error: &str,
) -> Result<()> {
    publish_topology_runtime_status(
        config,
        source_generation,
        None,
        None,
        false,
        Some(error.to_string()),
    )
}
