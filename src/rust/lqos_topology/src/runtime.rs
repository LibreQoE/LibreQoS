use anyhow::{Context, Result};
use lqos_bus::{BusReply, BusRequest};
use lqos_config::{
    TopologyAttachmentHealthStateFile, TopologyCanonicalStateFile,
    compute_topology_source_generation, load_config,
};
use lqos_overrides::TopologyOverridesFile;
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info, warn};

mod gate;
mod health;
mod probe;

use gate::{RoundHints, RuntimeBuildGate, topology_overrides_generation};
use health::{health_effective_signature, load_starting_health, now_unix, refresh_health_state};
use probe::probe_specs;

use crate::{
    build_effective_topology_artifacts_from_canonical_with_runtime_queue_context,
    load_canonical_topology_state, prepare_runtime_topology_editor_state_from_canonical,
    probe_specs_from_state, publish_effective_topology_artifacts,
    publish_topology_runtime_error_status,
};

type TopologyBusSender =
    tokio::sync::mpsc::Sender<(tokio::sync::oneshot::Sender<BusReply>, BusRequest)>;

fn run_round(
    bus_tx: TopologyBusSender,
    health_state: &mut TopologyAttachmentHealthStateFile,
    last_effective: &mut HashMap<String, Option<String>>,
    gate: &mut RuntimeBuildGate,
) -> Result<RoundHints> {
    let config = load_config().context("Unable to load config for topology runtime")?;
    let source_generation = compute_topology_source_generation(config.as_ref())
        .context("Unable to compute topology source generation")?;
    let overrides_generation = topology_overrides_generation(config.as_ref());
    let source_changed = gate.last_source_generation.as_deref() != Some(source_generation.as_str());
    let overrides_changed = gate.last_overrides_generation != Some(overrides_generation);
    let mut loaded_inputs: Option<(TopologyCanonicalStateFile, TopologyOverridesFile)> = None;
    if source_changed || overrides_changed || gate.cached_probe_specs.is_empty() {
        let canonical = load_canonical_topology_state(config.as_ref());
        let overrides =
            TopologyOverridesFile::load().context("Unable to load topology overrides file")?;
        let prepared = prepare_runtime_topology_editor_state_from_canonical(&canonical, &overrides);
        gate.cached_probe_specs = probe_specs_from_state(&prepared, &overrides);
        loaded_inputs = Some((canonical, overrides));
    }
    let specs = &gate.cached_probe_specs;
    let probes_enabled = specs.iter().any(|spec| spec.enabled);
    if probes_enabled {
        match probe_specs(bus_tx.clone(), specs, Duration::from_millis(750)) {
            Ok(probe_results) => {
                refresh_health_state(config.as_ref(), health_state, specs, &probe_results)?;
            }
            Err(err) => {
                warn!("Topology probe round could not query shared probe manager: {err:#}");
            }
        }
    } else {
        refresh_health_state(config.as_ref(), health_state, specs, &HashMap::new())?;
    }

    let next_signature = health_effective_signature(health_state);
    let retry_due = gate
        .next_error_retry_after_unix
        .is_some_and(|deadline| now_unix().is_some_and(|now| now >= deadline));
    let retry_pending = gate.next_error_retry_after_unix.is_some() && !retry_due;
    let source_or_health_changed = source_changed
        || overrides_changed
        || gate.last_health_effective_signature.as_ref() != Some(&next_signature);
    let should_publish =
        source_or_health_changed || (!retry_pending && (!gate.publish_completed || retry_due));
    if !should_publish {
        return Ok(RoundHints { probes_enabled });
    }

    let (canonical, overrides) = match loaded_inputs {
        Some(inputs) => inputs,
        None => {
            let canonical = load_canonical_topology_state(config.as_ref());
            let overrides =
                TopologyOverridesFile::load().context("Unable to load topology overrides file")?;
            (canonical, overrides)
        }
    };

    let artifacts = build_effective_topology_artifacts_from_canonical_with_runtime_queue_context(
        config.as_ref(),
        &canonical,
        &overrides,
        health_state,
    )
    .map_err(|errors| {
        anyhow::anyhow!(
            "Refusing to publish invalid effective topology: {}",
            errors.join(" | ")
        )
    })?;
    if let Err(err) =
        publish_effective_topology_artifacts(config.as_ref(), &artifacts, &source_generation)
            .context("Unable to publish effective topology artifacts")
    {
        let formatted = format!("{err:#}");
        if let Err(status_err) =
            publish_topology_runtime_error_status(config.as_ref(), &source_generation, &formatted)
        {
            warn!(
                "Unable to publish failed topology runtime status after publish error: {status_err:#}"
            );
        }
        let health = &config.integration_common.topology_attachment_health;
        let retry_delay = health
            .refresh_debounce_seconds
            .max(health.probe_interval_seconds.max(1));
        gate.last_source_generation = Some(source_generation.clone());
        gate.last_overrides_generation = Some(overrides_generation);
        gate.last_health_effective_signature = Some(next_signature);
        gate.next_error_retry_after_unix = now_unix().map(|now| now.saturating_add(retry_delay));
        return Err(err);
    }
    gate.last_source_generation = Some(source_generation.clone());
    gate.last_overrides_generation = Some(overrides_generation);
    gate.last_health_effective_signature = Some(next_signature);
    gate.publish_completed = true;
    gate.next_error_retry_after_unix = None;

    for node in &artifacts.effective.nodes {
        let next = node.effective_attachment_id.clone();
        let previous = last_effective.insert(node.node_id.clone(), next.clone());
        if previous != Some(next.clone()) {
            info!(
                node_id = %node.node_id,
                attachment = ?next,
                "Topology effective attachment updated"
            );
        }
    }

    Ok(RoundHints { probes_enabled })
}

/// Launches the threaded topology system.
pub fn start_topology_thread(
    bus_tx: tokio::sync::mpsc::Sender<(
        tokio::sync::oneshot::Sender<lqos_bus::BusReply>,
        BusRequest,
    )>,
) {
    let thread_result = std::thread::Builder::new()
        .name("topology".to_string())
        .spawn(|| {
            debug!("Starting Topology Thread");
            start_topology(bus_tx);
            warn!("Topology Thread Terminating");
        });

    if let Err(e) = thread_result {
        tracing::error!("Unable top start the topology thread: {e:?}");
    }
}

fn start_topology(bus_tx: TopologyBusSender) {
    let mut health_state = load_starting_health();
    let mut last_effective = HashMap::<String, Option<String>>::new();
    let mut build_gate = RuntimeBuildGate::default();

    loop {
        let round_hints = match run_round(
            bus_tx.clone(),
            &mut health_state,
            &mut last_effective,
            &mut build_gate,
        ) {
            Ok(hints) => hints,
            Err(err) => {
                if let Ok(config) = load_config()
                    && let Ok(source_generation) =
                        compute_topology_source_generation(config.as_ref())
                {
                    let formatted = format!("{err:#}");
                    if let Err(status_err) = publish_topology_runtime_error_status(
                        config.as_ref(),
                        &source_generation,
                        &formatted,
                    ) {
                        warn!(
                            "Unable to publish failed topology runtime status after round error: {status_err:#}"
                        );
                    }
                }
                warn!("Topology runtime round failed: {err:?}");
                RoundHints::default()
            }
        };

        let sleep_seconds = load_config()
            .ok()
            .map(|config| {
                let health = &config.integration_common.topology_attachment_health;
                let probe_interval = health.probe_interval_seconds.max(1);
                if round_hints.probes_enabled {
                    probe_interval
                } else {
                    probe_interval.max(health.refresh_debounce_seconds.max(5))
                }
            })
            .unwrap_or(1);
        std::thread::sleep(Duration::from_secs(sleep_seconds));
    }
}
