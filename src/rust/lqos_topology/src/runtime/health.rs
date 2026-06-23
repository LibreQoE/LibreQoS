use anyhow::{Context, Result};
use lqos_config::{
    TopologyAttachmentEndpointStatus, TopologyAttachmentHealthEntry,
    TopologyAttachmentHealthStateFile, TopologyAttachmentHealthStatus, load_config,
};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::{AttachmentProbeSpec, is_health_state_fresh, parse_probe_ip};

pub(super) fn now_unix() -> Option<u64> {
    lqos_utils::unix_time::unix_now().ok()
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

fn probeable_pair(local_ip: &str, remote_ip: &str) -> bool {
    parse_probe_ip(local_ip)
        .zip(parse_probe_ip(remote_ip))
        .is_some_and(|(local, remote)| local != remote)
}

fn base_health_entry(
    spec: &AttachmentProbeSpec,
    previous: Option<&TopologyAttachmentHealthEntry>,
) -> TopologyAttachmentHealthEntry {
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
    entry.probeable = probeable_pair(&spec.local_ip, &spec.remote_ip);
    entry
}

fn health_entry_for_unprobeable_spec(
    spec: &AttachmentProbeSpec,
    previous: Option<&TopologyAttachmentHealthEntry>,
) -> TopologyAttachmentHealthEntry {
    let mut entry = base_health_entry(spec, previous);
    if !spec.enabled {
        entry.status = TopologyAttachmentHealthStatus::Disabled;
        entry.reason = Some("Health probe disabled".to_string());
    } else {
        entry.status = TopologyAttachmentHealthStatus::ProbeUnavailable;
        entry.reason = Some(probe_unavailable_reason(&spec.local_ip, &spec.remote_ip));
    }
    entry.consecutive_misses = 0;
    entry.consecutive_successes = 0;
    entry.suppressed_until_unix = None;
    entry.endpoint_status = Vec::new();
    entry
}

pub(super) fn load_starting_health() -> TopologyAttachmentHealthStateFile {
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

fn build_health_entry(
    config: &lqos_config::Config,
    spec: &AttachmentProbeSpec,
    previous: Option<&TopologyAttachmentHealthEntry>,
    probe_result: Option<(bool, bool)>,
) -> TopologyAttachmentHealthEntry {
    let now = now_unix();
    let mut entry = base_health_entry(spec, previous);

    if !spec.enabled || !entry.probeable {
        return health_entry_for_unprobeable_spec(spec, previous);
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

fn build_unobserved_health_entry(
    spec: &AttachmentProbeSpec,
    previous: Option<&TopologyAttachmentHealthEntry>,
    reason: &str,
) -> TopologyAttachmentHealthEntry {
    let mut entry = base_health_entry(spec, previous);
    if !spec.enabled || !entry.probeable {
        return health_entry_for_unprobeable_spec(spec, previous);
    }

    let suppression_active = previous.is_some_and(|previous| {
        previous.status == TopologyAttachmentHealthStatus::Suppressed
            && previous
                .suppressed_until_unix
                .is_some_and(|deadline| now_unix().is_none_or(|now| now < deadline))
    });
    if suppression_active {
        entry.status = TopologyAttachmentHealthStatus::Suppressed;
        entry.reason = previous.and_then(|previous| previous.reason.clone());
    } else {
        entry.status = TopologyAttachmentHealthStatus::ProbeUnavailable;
        entry.reason = Some(reason.to_string());
        entry.suppressed_until_unix = None;
    }
    entry.consecutive_successes = 0;
    entry.endpoint_status = Vec::new();
    entry
}

fn save_health_entries(
    config: &lqos_config::Config,
    health_state: &mut TopologyAttachmentHealthStateFile,
    mut new_entries: Vec<TopologyAttachmentHealthEntry>,
) -> Result<bool> {
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

pub(super) fn refresh_health_state(
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
    let new_entries = specs
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
    save_health_entries(config, health_state, new_entries)
}

pub(super) fn mark_health_state_unobserved(
    config: &lqos_config::Config,
    health_state: &mut TopologyAttachmentHealthStateFile,
    specs: &[AttachmentProbeSpec],
    reason: &str,
) -> Result<bool> {
    let previous_by_pair = health_state
        .attachments
        .iter()
        .map(|entry| (entry.attachment_pair_id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let new_entries = specs
        .iter()
        .map(|spec| {
            build_unobserved_health_entry(
                spec,
                previous_by_pair.get(spec.pair_id.as_str()).copied(),
                reason,
            )
        })
        .collect::<Vec<_>>();
    save_health_entries(config, health_state, new_entries)
}

fn hash_health_status(status: TopologyAttachmentHealthStatus, hasher: &mut impl Hasher) {
    match status {
        TopologyAttachmentHealthStatus::Healthy => 0_u8.hash(hasher),
        TopologyAttachmentHealthStatus::Suppressed => 1_u8.hash(hasher),
        TopologyAttachmentHealthStatus::ProbeUnavailable => 2_u8.hash(hasher),
        TopologyAttachmentHealthStatus::Disabled => 3_u8.hash(hasher),
    }
}

fn hash_health_effective_entry(entry: &TopologyAttachmentHealthEntry, hasher: &mut impl Hasher) {
    entry.attachment_pair_id.hash(hasher);
    entry.attachment_id.hash(hasher);
    entry.child_node_id.hash(hasher);
    entry.parent_node_id.hash(hasher);
    entry.local_probe_ip.hash(hasher);
    entry.remote_probe_ip.hash(hasher);
    hash_health_status(entry.status, hasher);
    entry.probeable.hash(hasher);
    entry.enabled.hash(hasher);
    entry.suppressed_until_unix.hash(hasher);
}

pub(super) fn health_effective_signature(health_state: &TopologyAttachmentHealthStateFile) -> u64 {
    let mut hasher = DefaultHasher::new();
    for entry in &health_state.attachments {
        hash_health_effective_entry(entry, &mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::{
        TopologyAttachmentHealthEntry, build_unobserved_health_entry, health_effective_signature,
    };
    use crate::AttachmentProbeSpec;
    use lqos_config::{TopologyAttachmentHealthStateFile, TopologyAttachmentHealthStatus};

    fn probe_spec() -> AttachmentProbeSpec {
        AttachmentProbeSpec {
            pair_id: "pair-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            attachment_name: "Attachment 1".to_string(),
            node_id: "child-1".to_string(),
            node_name: "Child 1".to_string(),
            parent_node_id: "parent-1".to_string(),
            parent_node_name: "Parent 1".to_string(),
            local_ip: "192.0.2.1".to_string(),
            remote_ip: "192.0.2.2".to_string(),
            enabled: true,
        }
    }

    fn health_entry() -> TopologyAttachmentHealthEntry {
        TopologyAttachmentHealthEntry {
            attachment_pair_id: "pair-1".to_string(),
            attachment_id: Some("attachment-1".to_string()),
            child_node_id: Some("child-1".to_string()),
            parent_node_id: Some("parent-1".to_string()),
            local_probe_ip: Some("192.0.2.1".to_string()),
            remote_probe_ip: Some("192.0.2.2".to_string()),
            status: TopologyAttachmentHealthStatus::Healthy,
            probeable: true,
            enabled: true,
            consecutive_misses: 1,
            consecutive_successes: 2,
            last_success_unix: Some(10),
            endpoint_status: vec![],
            ..TopologyAttachmentHealthEntry::default()
        }
    }

    #[test]
    fn health_effective_signature_ignores_probe_counters_and_timestamps() {
        let first = TopologyAttachmentHealthStateFile {
            generated_unix: Some(1),
            attachments: vec![health_entry()],
            ..TopologyAttachmentHealthStateFile::default()
        };
        let mut second_entry = health_entry();
        second_entry.consecutive_misses = 4;
        second_entry.consecutive_successes = 5;
        second_entry.last_success_unix = Some(20);
        second_entry.last_failure_unix = Some(21);
        let second = TopologyAttachmentHealthStateFile {
            generated_unix: Some(2),
            attachments: vec![second_entry],
            ..TopologyAttachmentHealthStateFile::default()
        };

        assert_eq!(
            health_effective_signature(&first),
            health_effective_signature(&second)
        );
    }

    #[test]
    fn health_effective_signature_tracks_suppression_status() {
        let first = TopologyAttachmentHealthStateFile {
            attachments: vec![health_entry()],
            ..TopologyAttachmentHealthStateFile::default()
        };
        let mut suppressed = health_entry();
        suppressed.status = TopologyAttachmentHealthStatus::Suppressed;
        suppressed.suppressed_until_unix = Some(123);
        let second = TopologyAttachmentHealthStateFile {
            attachments: vec![suppressed],
            ..TopologyAttachmentHealthStateFile::default()
        };

        assert_ne!(
            health_effective_signature(&first),
            health_effective_signature(&second)
        );
    }

    #[test]
    fn unobserved_health_marks_probeable_pair_unavailable() {
        let entry = build_unobserved_health_entry(
            &probe_spec(),
            Some(&health_entry()),
            "Probe unavailable: shared probe manager unavailable",
        );

        assert_eq!(
            entry.status,
            TopologyAttachmentHealthStatus::ProbeUnavailable
        );
        assert_eq!(
            entry.reason.as_deref(),
            Some("Probe unavailable: shared probe manager unavailable")
        );
        assert!(entry.endpoint_status.is_empty());
    }

    #[test]
    fn unobserved_health_preserves_active_suppression() {
        let mut previous = health_entry();
        previous.status = TopologyAttachmentHealthStatus::Suppressed;
        previous.reason = Some("2 missed probes".to_string());
        previous.suppressed_until_unix = Some(u64::MAX);

        let entry = build_unobserved_health_entry(
            &probe_spec(),
            Some(&previous),
            "Probe unavailable: shared probe manager unavailable",
        );

        assert_eq!(entry.status, TopologyAttachmentHealthStatus::Suppressed);
        assert_eq!(entry.reason.as_deref(), Some("2 missed probes"));
    }
}
