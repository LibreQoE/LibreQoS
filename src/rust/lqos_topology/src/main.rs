use anyhow::{Context, Result};
use lqos_bus::{BusRequest, BusResponse, bus_request};
use lqos_config::{
    TOPOLOGY_EFFECTIVE_NETWORK_FILENAME, TopologyAttachmentEndpointStatus,
    TopologyAttachmentHealthEntry, TopologyAttachmentHealthStateFile,
    TopologyAttachmentHealthStatus, TopologyEditorStateFile, load_config,
    topology_effective_network_path,
};
use lqos_overrides::TopologyOverridesFile;
use lqos_probe::{ProbeClass, ProbeRequest};
use lqos_topology::{
    AttachmentProbeSpec, apply_effective_topology_to_network_json, compute_effective_state,
    is_health_state_fresh, merged_topology_state, probe_specs_from_state,
};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::net::IpAddr;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

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

fn atomic_write_json(path: &Path, value: &Value) -> Result<()> {
    let raw = serde_json::to_string_pretty(value)?;
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
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

fn load_canonical_network_json() -> Option<Value> {
    let config = load_config().ok()?;
    let base = Path::new(&config.lqos_directory);
    let path = if config
        .long_term_stats
        .enable_insight_topology
        .unwrap_or(false)
    {
        let insight = base.join("network.insight.json");
        if insight.exists() {
            insight
        } else {
            base.join("network.json")
        }
    } else {
        base.join("network.json")
    };
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
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
        if let Some(ip) = parse_probe_ip(&spec.local_ip) {
            probe_positions.push((spec.pair_id.clone(), 0_usize));
            probe_requests.push(ProbeRequest::reachability(
                ip.to_string(),
                ProbeClass::TopologyAttachment,
                timeout,
            ));
        }
        if let Some(ip) = parse_probe_ip(&spec.remote_ip) {
            probe_positions.push((spec.pair_id.clone(), 1_usize));
            probe_requests.push(ProbeRequest::reachability(
                ip.to_string(),
                ProbeClass::TopologyAttachment,
                timeout,
            ));
        }
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
) -> Result<()> {
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
    health_state.schema_version = 1;
    health_state.generated_unix = now_unix();
    health_state.attachments = new_entries;
    health_state
        .save(config)
        .context("Unable to save topology attachment health state")
}

async fn run_round(
    health_state: &mut TopologyAttachmentHealthStateFile,
    last_effective: &mut HashMap<String, Option<String>>,
) -> Result<()> {
    let config = load_config().context("Unable to load config for topology runtime")?;
    let canonical = TopologyEditorStateFile::load_with_legacy_fallback(config.as_ref())
        .context("Unable to load canonical topology editor state")?;
    let overrides =
        TopologyOverridesFile::load().context("Unable to load topology overrides file")?;

    let initial_effective =
        compute_effective_state(config.as_ref(), &canonical, &overrides, health_state);
    let initial_ui_state = merged_topology_state(
        config.as_ref(),
        &canonical,
        &overrides,
        health_state,
        &initial_effective,
    );
    let specs = probe_specs_from_state(&initial_ui_state, &overrides);
    match probe_specs(&specs, Duration::from_millis(750)).await {
        Ok(probe_results) => {
            refresh_health_state(config.as_ref(), health_state, &specs, &probe_results)?;
        }
        Err(err) => {
            warn!("Topology probe round could not query shared probe manager: {err:#}");
        }
    }

    let effective = compute_effective_state(config.as_ref(), &canonical, &overrides, health_state);
    effective
        .save(config.as_ref())
        .context("Unable to save topology effective state")?;
    let ui_state = merged_topology_state(
        config.as_ref(),
        &canonical,
        &overrides,
        health_state,
        &effective,
    );

    if let Some(canonical_network) = load_canonical_network_json() {
        let effective_network = apply_effective_topology_to_network_json(
            config.as_ref(),
            &canonical_network,
            &ui_state,
            &effective,
        );
        let effective_path = topology_effective_network_path(config.as_ref());
        atomic_write_json(&effective_path, &effective_network)
            .with_context(|| format!("Unable to save {}", TOPOLOGY_EFFECTIVE_NETWORK_FILENAME))?;
    }

    for node in &effective.nodes {
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

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let mut health_state = load_starting_health();
    let mut last_effective = HashMap::<String, Option<String>>::new();

    loop {
        if let Err(err) = run_round(&mut health_state, &mut last_effective).await {
            warn!("Topology runtime round failed: {err:?}");
        }

        let sleep_seconds = load_config()
            .ok()
            .map(|config| {
                config
                    .integration_common
                    .topology_attachment_health
                    .probe_interval_seconds
                    .max(1)
            })
            .unwrap_or(1);
        tokio::time::sleep(Duration::from_secs(sleep_seconds)).await;
    }
}
