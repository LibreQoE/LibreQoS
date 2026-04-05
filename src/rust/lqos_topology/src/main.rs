use anyhow::{Context, Result};
use lqos_config::{
    TOPOLOGY_EFFECTIVE_NETWORK_FILENAME, TopologyAttachmentEndpointStatus,
    TopologyAttachmentHealthEntry, TopologyAttachmentHealthStateFile,
    TopologyAttachmentHealthStatus, TopologyEditorStateFile, load_config,
    topology_effective_network_path,
};
use lqos_topology::{
    AttachmentProbeSpec, apply_effective_topology_to_network_json, compute_effective_state,
    is_health_state_fresh, merged_topology_state, probe_specs_from_state,
};
use lqos_overrides::TopologyOverridesFile;
use rand::random;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use surge_ping::{Client, Config as PingConfig, ICMP, IcmpPacket, PingIdentifier, PingSequence};
use tokio::sync::Semaphore;
use tracing::{info, warn};

fn now_unix() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
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
    if local == remote {
        return "Probe unavailable: local and remote probe IPs are identical".to_string();
    }
    if local.parse::<IpAddr>().is_err() && remote.parse::<IpAddr>().is_err() {
        return "Probe unavailable: local and remote probe IPs are invalid".to_string();
    }
    if local.parse::<IpAddr>().is_err() {
        return "Probe unavailable: local management IP is invalid".to_string();
    }
    if remote.parse::<IpAddr>().is_err() {
        return "Probe unavailable: remote management IP is invalid".to_string();
    }
    "Probe unavailable".to_string()
}

fn load_canonical_network_json() -> Option<Value> {
    let config = load_config().ok()?;
    let base = Path::new(&config.lqos_directory);
    let path = if config.long_term_stats.enable_insight_topology.unwrap_or(false) {
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

async fn ping_once(ip: IpAddr, timeout: Duration) -> bool {
    let client = match ip {
        IpAddr::V4(_) => Client::new(&PingConfig::default()),
        IpAddr::V6(_) => Client::new(&PingConfig::builder().kind(ICMP::V6).build()),
    };
    let Ok(client) = client else {
        return false;
    };
    let payload = [0_u8; 56];
    let mut pinger = client.pinger(ip, PingIdentifier(random())).await;
    pinger.timeout(timeout);
    matches!(
        pinger.ping(PingSequence(0), &payload).await,
        Ok((IcmpPacket::V4(..), _)) | Ok((IcmpPacket::V6(..), _))
    )
}

async fn probe_specs(
    specs: &[AttachmentProbeSpec],
    timeout: Duration,
) -> HashMap<String, (bool, bool)> {
    let semaphore = Arc::new(Semaphore::new(256));
    let mut join_set = tokio::task::JoinSet::new();
    for spec in specs {
        let pair_id = spec.pair_id.clone();
        let local_ip = spec.local_ip.clone();
        let local_semaphore = semaphore.clone();
        join_set.spawn(async move {
            let _permit = local_semaphore.acquire_owned().await.ok();
            let reachable = match local_ip.parse::<IpAddr>() {
                Ok(ip) => ping_once(ip, timeout).await,
                Err(_) => false,
            };
            (pair_id, 0_usize, reachable)
        });

        let pair_id = spec.pair_id.clone();
        let remote_ip = spec.remote_ip.clone();
        let remote_semaphore = semaphore.clone();
        join_set.spawn(async move {
            let _permit = remote_semaphore.acquire_owned().await.ok();
            let reachable = match remote_ip.parse::<IpAddr>() {
                Ok(ip) => ping_once(ip, timeout).await,
                Err(_) => false,
            };
            (pair_id, 1_usize, reachable)
        });
    }

    let mut results = HashMap::<String, (bool, bool)>::new();
    while let Some(result) = join_set.join_next().await {
        let Ok((pair_id, endpoint_index, reachable)) = result else {
            continue;
        };
        let entry = results.entry(pair_id).or_insert((false, false));
        if endpoint_index == 0 {
            entry.0 = reachable;
        } else {
            entry.1 = reachable;
        }
    }
    results
}

fn build_health_entry(
    config: &lqos_config::Config,
    spec: &AttachmentProbeSpec,
    previous: Option<&TopologyAttachmentHealthEntry>,
    probe_result: Option<(bool, bool)>,
) -> TopologyAttachmentHealthEntry {
    let now = now_unix();
    let probeable = spec.local_ip.parse::<IpAddr>().is_ok()
        && spec.remote_ip.parse::<IpAddr>().is_ok()
        && spec.local_ip != spec.remote_ip;
    let mut entry = previous.cloned().unwrap_or_else(|| TopologyAttachmentHealthEntry {
        attachment_pair_id: spec.pair_id.clone(),
        ..TopologyAttachmentHealthEntry::default()
    });
    entry.attachment_pair_id = spec.pair_id.clone();
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

async fn run_round(
    health_state: &mut TopologyAttachmentHealthStateFile,
    last_effective: &mut HashMap<String, Option<String>>,
) -> Result<()> {
    let config = load_config().context("Unable to load config for topology runtime")?;
    let canonical = TopologyEditorStateFile::load_with_legacy_fallback(config.as_ref())
        .context("Unable to load canonical topology editor state")?;
    let overrides =
        TopologyOverridesFile::load().context("Unable to load topology overrides file")?;

    let initial_effective = compute_effective_state(config.as_ref(), &canonical, &overrides, health_state);
    let initial_ui_state =
        merged_topology_state(config.as_ref(), &canonical, &overrides, health_state, &initial_effective);
    let specs = probe_specs_from_state(&initial_ui_state, &overrides);
    let probe_results = probe_specs(&specs, Duration::from_millis(750)).await;

    let previous_by_pair = health_state
        .attachments
        .iter()
        .map(|entry| (entry.attachment_pair_id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let mut new_entries = specs
        .iter()
        .map(|spec| {
            build_health_entry(
                config.as_ref(),
                spec,
                previous_by_pair.get(spec.pair_id.as_str()).copied(),
                probe_results.get(&spec.pair_id).copied(),
            )
        })
        .collect::<Vec<_>>();
    new_entries.sort_unstable_by(|left, right| left.attachment_pair_id.cmp(&right.attachment_pair_id));
    health_state.schema_version = 1;
    health_state.generated_unix = now_unix();
    health_state.attachments = new_entries;
    health_state
        .save(config.as_ref())
        .context("Unable to save topology attachment health state")?;

    let effective = compute_effective_state(config.as_ref(), &canonical, &overrides, health_state);
    effective
        .save(config.as_ref())
        .context("Unable to save topology effective state")?;
    let ui_state =
        merged_topology_state(config.as_ref(), &canonical, &overrides, health_state, &effective);

    if let Some(canonical_network) = load_canonical_network_json() {
        let effective_network =
            apply_effective_topology_to_network_json(&canonical_network, &ui_state, &effective);
        let effective_path = topology_effective_network_path(config.as_ref());
        atomic_write_json(&effective_path, &effective_network).with_context(|| {
            format!(
                "Unable to save {}",
                TOPOLOGY_EFFECTIVE_NETWORK_FILENAME
            )
        })?;
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
