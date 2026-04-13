use anyhow::{Context, Result};
use lqos_bus::{BusRequest, BusResponse, bus_request};
use lqos_config::{
    TopologyAttachmentEndpointStatus, TopologyAttachmentHealthEntry,
    TopologyAttachmentHealthStateFile, TopologyAttachmentHealthStatus,
    compute_topology_source_generation, load_config,
};
use lqos_overrides::TopologyOverridesFile;
use lqos_probe::{ProbeClass, ProbeRequest};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

use crate::{
    AttachmentProbeSpec, build_effective_topology_artifacts_from_canonical, is_health_state_fresh,
    load_canonical_topology_state, prepare_runtime_topology_editor_state_from_canonical,
    probe_specs_from_state, publish_effective_topology_artifacts,
    publish_topology_runtime_error_status,
};

const TOPOLOGY_PROBE_MAX_AGE_MS: u64 = 250;

fn now_unix() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn parse_probe_ip(raw: &str) -> Option<IpAddr> {
    raw.trim()
        .split('/')
        .next()
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<IpAddr>().ok())
}

fn probe_unavailable_reason(local_ip: &str, remote_ip: &str) -> String {
    let local = local_ip.trim();
    let remote = remote_ip.trim();

    if local.is_empty() && remote.is_empty() {
        return "Probe unavailable: missing local and remote management IPs".to_string();
    }
    if local.is_empty() {
        return "Probe unavailable: missing local management IP".to_string();
    }
    if remote.is_empty() {
        return "Probe unavailable: missing remote management IP".to_string();
    }
    if parse_probe_ip(local)
        .zip(parse_probe_ip(remote))
        .is_some_and(|(local, remote)| local == remote)
    {
        return "Probe unavailable: local and remote probe IPs are identical".to_string();
    }
    if parse_probe_ip(local).is_none() && parse_probe_ip(remote).is_none() {
        return "Probe unavailable: local and remote probe IPs are invalid".to_string();
    }
    if parse_probe_ip(local).is_none() {
        return "Probe unavailable: local management IP is invalid".to_string();
    }
    if parse_probe_ip(remote).is_none() {
        return "Probe unavailable: remote management IP is invalid".to_string();
    }
    "Probe unavailable".to_string()
}

fn load_starting_health() -> TopologyAttachmentHealthStateFile {
    let Ok(config) = load_config() else {
        return TopologyAttachmentHealthStateFile::default();
    };
    let Ok(health) = TopologyAttachmentHealthStateFile::load(config.as_ref()) else {
        return TopologyAttachmentHealthStateFile::default();
    };
    if is_health_state_fresh(config.as_ref(), &health) {
        health
    } else {
        TopologyAttachmentHealthStateFile::default()
    }
}

async fn probe_specs(
    specs: &[AttachmentProbeSpec],
    timeout: Duration,
) -> Result<HashMap<String, (bool, bool)>> {
    let mut probe_requests = Vec::new();
    let mut probe_positions = Vec::new();
    for spec in specs {
        if !spec.enabled {
            continue;
        }
        let Some(local_ip) = parse_probe_ip(&spec.local_ip) else {
            continue;
        };
        let Some(remote_ip) = parse_probe_ip(&spec.remote_ip) else {
            continue;
        };
        if local_ip == remote_ip {
            continue;
        }

        probe_positions.push((spec.pair_id.clone(), 0_usize));
        probe_requests.push(ProbeRequest::reachability(
            local_ip.to_string(),
            ProbeClass::TopologyAttachment,
            timeout,
        ));
        probe_positions.push((spec.pair_id.clone(), 1_usize));
        probe_requests.push(ProbeRequest::reachability(
            remote_ip.to_string(),
            ProbeClass::TopologyAttachment,
            timeout,
        ));
    }

    if probe_requests.is_empty() {
        return Ok(HashMap::new());
    }

    let responses = bus_request(vec![BusRequest::ProbeBatch {
        requests: probe_requests,
        max_age_ms: TOPOLOGY_PROBE_MAX_AGE_MS,
    }])
    .await
    .map_err(|err| anyhow::anyhow!("unable to query shared probe manager: {err}"))?;
    let Some(response) = responses.into_iter().next() else {
        return Err(anyhow::anyhow!(
            "shared probe manager returned no bus response for topology batch"
        ));
    };

    let mut results = HashMap::<String, (bool, bool)>::new();
    match response {
        BusResponse::ProbeObservations(observations) => {
            for ((pair_id, endpoint_index), observation) in
                probe_positions.into_iter().zip(observations)
            {
                let entry = results.entry(pair_id).or_insert((false, false));
                if endpoint_index == 0 {
                    entry.0 = observation.reachable;
                } else {
                    entry.1 = observation.reachable;
                }
            }
            Ok(results)
        }
        BusResponse::Fail(message) => Err(anyhow::anyhow!(
            "shared probe manager rejected topology batch: {message}"
        )),
        other => Err(anyhow::anyhow!(
            "unexpected response from shared probe manager: {other:?}"
        )),
    }
}

fn build_health_entry(
    config: &lqos_config::Config,
    spec: &AttachmentProbeSpec,
    previous: Option<&TopologyAttachmentHealthEntry>,
    probe_result: Option<(bool, bool)>,
) -> TopologyAttachmentHealthEntry {
    let now = now_unix();
    let probeable = parse_probe_ip(&spec.local_ip)
        .zip(parse_probe_ip(&spec.remote_ip))
        .is_some_and(|(local, remote)| local != remote);
    let mut entry = previous
        .cloned()
        .unwrap_or_else(|| TopologyAttachmentHealthEntry {
            attachment_pair_id: spec.pair_id.clone(),
            ..TopologyAttachmentHealthEntry::default()
        });
    entry.attachment_pair_id = spec.pair_id.clone();
    entry.attachment_id = Some(spec.attachment_id.clone());
    entry.attachment_name = Some(spec.attachment_name.clone());
    entry.child_node_id = Some(spec.node_id.clone());
    entry.child_node_name = Some(spec.node_name.clone());
    entry.parent_node_id = Some(spec.parent_node_id.clone());
    entry.parent_node_name = Some(spec.parent_node_name.clone());
    entry.local_probe_ip = Some(spec.local_ip.clone());
    entry.remote_probe_ip = Some(spec.remote_ip.clone());
    entry.enabled = spec.enabled;
    entry.probeable = probeable;

    if !spec.enabled {
        entry.status = TopologyAttachmentHealthStatus::Disabled;
        entry.reason = Some("Health probe disabled".to_string());
        entry.consecutive_misses = 0;
        entry.consecutive_successes = 0;
        entry.suppressed_until_unix = None;
        entry.endpoint_status = Vec::new();
        return entry;
    }

    if !probeable {
        entry.status = TopologyAttachmentHealthStatus::ProbeUnavailable;
        entry.reason = Some(probe_unavailable_reason(&spec.local_ip, &spec.remote_ip));
        entry.consecutive_misses = 0;
        entry.consecutive_successes = 0;
        entry.suppressed_until_unix = None;
        entry.endpoint_status = Vec::new();
        return entry;
    }

    let (local_reachable, remote_reachable) = probe_result.unwrap_or((false, false));
    entry.endpoint_status = vec![
        TopologyAttachmentEndpointStatus {
            attachment_id: spec.attachment_id.clone(),
            ip: spec.local_ip.clone(),
            reachable: local_reachable,
        },
        TopologyAttachmentEndpointStatus {
            attachment_id: format!("{}:remote", spec.attachment_id),
            ip: spec.remote_ip.clone(),
            reachable: remote_reachable,
        },
    ];

    if local_reachable && remote_reachable {
        entry.consecutive_misses = 0;
        entry.consecutive_successes = entry.consecutive_successes.saturating_add(1);
        entry.last_success_unix = now;
        let hold_down_active = entry
            .suppressed_until_unix
            .is_some_and(|deadline| now.is_some_and(|ts| ts < deadline));
        if entry.status == TopologyAttachmentHealthStatus::Suppressed
            && (hold_down_active
                || entry.consecutive_successes
                    < config
                        .integration_common
                        .topology_attachment_health
                        .clear_after_successes)
        {
            entry.reason = Some("Recovery hold-down active".to_string());
        } else {
            entry.status = TopologyAttachmentHealthStatus::Healthy;
            entry.reason = None;
            entry.suppressed_until_unix = None;
        }
        return entry;
    }

    entry.consecutive_successes = 0;
    entry.consecutive_misses = entry.consecutive_misses.saturating_add(1);
    entry.last_failure_unix = now;
    if entry.consecutive_misses
        >= config
            .integration_common
            .topology_attachment_health
            .fail_after_missed
    {
        entry.status = TopologyAttachmentHealthStatus::Suppressed;
        entry.reason = Some(format!("{} missed probes", entry.consecutive_misses));
        entry.suppressed_until_unix = now.map(|ts| {
            ts.saturating_add(
                config
                    .integration_common
                    .topology_attachment_health
                    .hold_down_seconds,
            )
        });
    } else {
        entry.status = TopologyAttachmentHealthStatus::Healthy;
        entry.reason = None;
        entry.suppressed_until_unix = None;
    }
    entry
}

fn refresh_health_state(
    config: &lqos_config::Config,
    health_state: &mut TopologyAttachmentHealthStateFile,
    specs: &[AttachmentProbeSpec],
    probe_results: &HashMap<String, (bool, bool)>,
) -> Result<bool> {
    let previous_by_pair = health_state
        .attachments
        .iter()
        .map(|entry| (entry.attachment_pair_id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let mut new_entries = specs
        .iter()
        .map(|spec| {
            build_health_entry(
                config,
                spec,
                previous_by_pair.get(spec.pair_id.as_str()).copied(),
                probe_results.get(&spec.pair_id).copied(),
            )
        })
        .collect::<Vec<_>>();
    new_entries
        .sort_unstable_by(|left, right| left.attachment_pair_id.cmp(&right.attachment_pair_id));
    let mut next_state = health_state.clone();
    next_state.schema_version = 1;
    next_state.attachments = new_entries;

    let mut previous_for_compare = health_state.clone();
    previous_for_compare.generated_unix = None;
    let mut next_for_compare = next_state.clone();
    next_for_compare.generated_unix = None;
    if previous_for_compare == next_for_compare {
        return Ok(false);
    }

    next_state.generated_unix = now_unix();
    next_state
        .save(config)
        .context("Unable to save topology attachment health state")?;
    *health_state = next_state;
    Ok(true)
}

#[derive(Clone, Copy, Debug, Default)]
struct RoundHints {
    probes_enabled: bool,
}

async fn run_round(
    health_state: &mut TopologyAttachmentHealthStateFile,
    last_effective: &mut HashMap<String, Option<String>>,
) -> Result<RoundHints> {
    let config = load_config().context("Unable to load config for topology runtime")?;
    let source_generation = compute_topology_source_generation(config.as_ref())
        .context("Unable to compute topology source generation")?;
    let canonical = load_canonical_topology_state(config.as_ref());
    let overrides =
        TopologyOverridesFile::load().context("Unable to load topology overrides file")?;
    let prepared = prepare_runtime_topology_editor_state_from_canonical(&canonical, &overrides);
    let specs = probe_specs_from_state(&prepared, &overrides);
    let probes_enabled = specs.iter().any(|spec| spec.enabled);
    if probes_enabled {
        match probe_specs(&specs, Duration::from_millis(750)).await {
            Ok(probe_results) => {
                refresh_health_state(config.as_ref(), health_state, &specs, &probe_results)?;
            }
            Err(err) => {
                warn!("Topology probe round could not query shared probe manager: {err:#}");
            }
        }
    } else {
        refresh_health_state(config.as_ref(), health_state, &specs, &HashMap::new())?;
    }

    let artifacts = build_effective_topology_artifacts_from_canonical(
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
        return Err(err);
    }

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

/// Starts the long-running topology runtime loop.
///
/// Side effects: reads topology/config inputs, issues probe batches through the
/// local LibreQoS bus, and continuously publishes runtime topology artifacts and
/// status files for the scheduler and UI.
pub async fn start_topology() {
    let mut health_state = load_starting_health();
    let mut last_effective = HashMap::<String, Option<String>>::new();

    loop {
        let round_hints = match run_round(&mut health_state, &mut last_effective).await {
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
        tokio::time::sleep(Duration::from_secs(sleep_seconds)).await;
    }
}
