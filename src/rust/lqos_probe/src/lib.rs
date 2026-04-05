//! Shared active probe provider for LibreQoS.
//!
//! This crate centralizes ICMP execution, target normalization, timeout policy,
//! cache freshness, and configuration gates such as `disable_icmp_ping`.

#![warn(missing_docs)]

use allocative::Allocative;
use lqos_config::load_config;
use rand::random;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use surge_ping::{Client, Config as PingConfig, ICMP, IcmpPacket, PingIdentifier, PingSequence};
use thiserror::Error;
use tokio::sync::{Semaphore, mpsc, oneshot};
use tracing::warn;

/// Logical consumer class for a probe request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, Allocative)]
pub enum ProbeClass {
    /// Topology attachment-health and failover work.
    TopologyAttachment,
    /// StormGuard active RTT sampling.
    Stormguard,
    /// Operator-facing UI monitors and diagnostics.
    UiMonitor,
    /// Miscellaneous one-off diagnostics.
    Diagnostic,
}

/// Probe measurement type.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, Allocative)]
pub enum ProbeKind {
    /// Probe for simple reachability.
    Reachability,
    /// Probe for round-trip time.
    RoundTripTime,
}

/// One probe request handled by the shared provider.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, Allocative)]
pub struct ProbeRequest {
    /// Raw target as requested by the caller.
    pub target: String,
    /// Requested measurement type.
    pub kind: ProbeKind,
    /// ICMP timeout to use for this request.
    pub timeout: Duration,
    /// Logical consumer class.
    pub class: ProbeClass,
}

impl ProbeRequest {
    /// Builds a reachability request.
    pub fn reachability(target: impl Into<String>, class: ProbeClass, timeout: Duration) -> Self {
        Self {
            target: target.into(),
            kind: ProbeKind::Reachability,
            timeout,
            class,
        }
    }

    /// Builds an RTT request.
    pub fn round_trip_time(
        target: impl Into<String>,
        class: ProbeClass,
        timeout: Duration,
    ) -> Self {
        Self {
            target: target.into(),
            kind: ProbeKind::RoundTripTime,
            timeout,
            class,
        }
    }
}

/// Result of a single probe request.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, Allocative)]
pub struct ProbeObservation {
    /// Raw target requested by the caller.
    pub requested_target: String,
    /// Canonical cache key for the target.
    pub normalized_target: String,
    /// Resolved IP when available.
    pub resolved_ip: Option<String>,
    /// Logical consumer class that requested the observation.
    pub class: ProbeClass,
    /// Requested measurement type.
    pub kind: ProbeKind,
    /// Timestamp of the observation in Unix milliseconds.
    pub observed_at_unix_ms: u64,
    /// Whether the target responded to the probe.
    pub reachable: bool,
    /// Observed RTT in milliseconds, when available.
    pub rtt_ms: Option<f64>,
    /// Error or failure reason, when present.
    pub error: Option<String>,
}

/// Configuration for the shared probe manager.
#[derive(Clone, Copy, Debug)]
pub struct ProbeManagerConfig {
    /// Maximum number of in-flight ICMP probes the manager may execute concurrently.
    pub max_concurrent_probes: usize,
    /// Channel capacity used between clients and the manager task.
    pub command_buffer: usize,
}

impl Default for ProbeManagerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_probes: 256,
            command_buffer: 128,
        }
    }
}

/// Handle used by consumers to issue probe requests to the manager.
#[derive(Clone, Debug)]
pub struct ProbeClient {
    tx: mpsc::Sender<ProbeCommand>,
}

impl ProbeClient {
    /// Issues a batch of probe requests and returns the resulting observations.
    pub async fn probe_batch(
        &self,
        requests: Vec<ProbeRequest>,
        max_age: Duration,
    ) -> Result<Vec<ProbeObservation>, ProbeClientError> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ProbeCommand::ProbeBatch {
                requests,
                max_age,
                reply: reply_tx,
            })
            .await
            .map_err(|_| ProbeClientError::ManagerUnavailable)?;
        reply_rx
            .await
            .map_err(|_| ProbeClientError::ManagerUnavailable)
    }

    /// Probes a batch of targets for reachability.
    pub async fn probe_reachability_batch(
        &self,
        targets: impl IntoIterator<Item = String>,
        class: ProbeClass,
        timeout: Duration,
        max_age: Duration,
    ) -> Result<Vec<ProbeObservation>, ProbeClientError> {
        let requests = targets
            .into_iter()
            .map(|target| ProbeRequest::reachability(target, class, timeout))
            .collect();
        self.probe_batch(requests, max_age).await
    }

    /// Probes a single target for RTT.
    pub async fn probe_round_trip_time(
        &self,
        target: impl Into<String>,
        class: ProbeClass,
        timeout: Duration,
        max_age: Duration,
    ) -> Result<ProbeObservation, ProbeClientError> {
        let target = target.into();
        let mut results = self
            .probe_batch(
                vec![ProbeRequest::round_trip_time(target, class, timeout)],
                max_age,
            )
            .await?;
        results.pop().ok_or(ProbeClientError::EmptyResponse)
    }
}

/// Errors returned by the shared probe client.
#[derive(Debug, Error)]
pub enum ProbeClientError {
    /// The background manager task is not available.
    #[error("shared probe manager is unavailable")]
    ManagerUnavailable,
    /// The manager returned no observations for the request.
    #[error("shared probe manager returned no observations")]
    EmptyResponse,
}

/// Background shared probe manager.
pub struct ProbeManager;

impl ProbeManager {
    /// Starts the background probe manager task.
    ///
    /// Side effects: spawns a Tokio task that executes ICMP probes in response
    /// to client requests.
    #[must_use]
    pub fn spawn(config: ProbeManagerConfig) -> ProbeClient {
        let (tx, rx) = mpsc::channel(config.command_buffer);
        tokio::spawn(async move {
            let mut state = ProbeManagerState::new(config);
            state.run(rx).await;
        });
        ProbeClient { tx }
    }
}

#[derive(Clone, Debug)]
struct ProbeCacheEntry {
    normalized_target: String,
    resolved_ip: Option<String>,
    kind: ProbeKind,
    observed_at_unix_ms: u64,
    reachable: bool,
    rtt_ms: Option<f64>,
    error: Option<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ProbeCacheKey {
    normalized_target: String,
    kind: ProbeKind,
    timeout_ms: u64,
}

enum ProbeCommand {
    ProbeBatch {
        requests: Vec<ProbeRequest>,
        max_age: Duration,
        reply: oneshot::Sender<Vec<ProbeObservation>>,
    },
}

struct ProbeManagerState {
    cache: HashMap<ProbeCacheKey, ProbeCacheEntry>,
    semaphore: std::sync::Arc<Semaphore>,
}

impl ProbeManagerState {
    fn new(config: ProbeManagerConfig) -> Self {
        Self {
            cache: HashMap::new(),
            semaphore: std::sync::Arc::new(Semaphore::new(config.max_concurrent_probes)),
        }
    }

    async fn run(&mut self, mut rx: mpsc::Receiver<ProbeCommand>) {
        while let Some(command) = rx.recv().await {
            match command {
                ProbeCommand::ProbeBatch {
                    requests,
                    max_age,
                    reply,
                } => {
                    let observations = self.handle_batch(requests, max_age).await;
                    let _ = reply.send(observations);
                }
            }
        }
    }

    async fn handle_batch(
        &mut self,
        requests: Vec<ProbeRequest>,
        max_age: Duration,
    ) -> Vec<ProbeObservation> {
        let icmp_disabled = icmp_disabled();
        let now = now_unix_ms();
        let mut results: Vec<Option<ProbeObservation>> = vec![None; requests.len()];
        let mut missing_by_key: HashMap<ProbeCacheKey, ProbeRequest> = HashMap::new();
        let mut request_positions: HashMap<ProbeCacheKey, Vec<usize>> = HashMap::new();

        for (index, request) in requests.iter().enumerate() {
            let Some(normalized_target) = normalize_target(&request.target) else {
                results[index] = Some(invalid_target_observation(request, now));
                continue;
            };

            let key = ProbeCacheKey {
                normalized_target,
                kind: request.kind,
                timeout_ms: duration_to_millis(request.timeout),
            };

            if icmp_disabled {
                results[index] = Some(disabled_observation(request, &key, now));
                continue;
            }

            if let Some(entry) = self.cache.get(&key)
                && now.saturating_sub(entry.observed_at_unix_ms) <= duration_to_millis(max_age)
            {
                results[index] = Some(observation_from_entry(request, entry));
                continue;
            }

            missing_by_key
                .entry(key.clone())
                .or_insert_with(|| request.clone());
            request_positions.entry(key).or_default().push(index);
        }

        let mut join_set = tokio::task::JoinSet::new();
        for (key, request) in missing_by_key {
            let semaphore = self.semaphore.clone();
            join_set.spawn(async move {
                let _permit = semaphore.acquire_owned().await.ok();
                let observation = execute_probe(&request, &key.normalized_target).await;
                (key, observation)
            });
        }

        let mut completed = HashMap::<ProbeCacheKey, ProbeCacheEntry>::new();
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok((key, entry)) => {
                    self.cache.insert(key.clone(), entry.clone());
                    completed.insert(key, entry);
                }
                Err(err) => {
                    warn!("probe worker join failed: {err}");
                }
            }
        }

        for (key, positions) in request_positions {
            let Some(entry) = completed.get(&key).or_else(|| self.cache.get(&key)) else {
                continue;
            };
            for index in positions {
                if let Some(request) = requests.get(index) {
                    results[index] = Some(observation_from_entry(request, entry));
                }
            }
        }

        results
            .into_iter()
            .zip(requests.into_iter())
            .map(|(result, request)| {
                result.unwrap_or_else(|| {
                    failed_observation(
                        &request,
                        &normalize_target(&request.target)
                            .unwrap_or_else(|| request.target.trim().to_string()),
                        None,
                        now_unix_ms(),
                        "probe manager returned no result".to_string(),
                    )
                })
            })
            .collect()
    }
}

fn duration_to_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn normalize_target(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Some(ip.to_string());
    }
    Some(trimmed.to_ascii_lowercase())
}

fn icmp_disabled() -> bool {
    load_config()
        .ok()
        .and_then(|cfg| cfg.disable_icmp_ping)
        .unwrap_or(false)
}

fn invalid_target_observation(
    request: &ProbeRequest,
    observed_at_unix_ms: u64,
) -> ProbeObservation {
    ProbeObservation {
        requested_target: request.target.clone(),
        normalized_target: request.target.trim().to_string(),
        resolved_ip: None,
        class: request.class,
        kind: request.kind,
        observed_at_unix_ms,
        reachable: false,
        rtt_ms: None,
        error: Some("probe target is empty".to_string()),
    }
}

fn disabled_observation(
    request: &ProbeRequest,
    key: &ProbeCacheKey,
    observed_at_unix_ms: u64,
) -> ProbeObservation {
    ProbeObservation {
        requested_target: request.target.clone(),
        normalized_target: key.normalized_target.clone(),
        resolved_ip: None,
        class: request.class,
        kind: request.kind,
        observed_at_unix_ms,
        reachable: false,
        rtt_ms: None,
        error: Some("ICMP ping is disabled in the configuration".to_string()),
    }
}

fn observation_from_entry(request: &ProbeRequest, entry: &ProbeCacheEntry) -> ProbeObservation {
    ProbeObservation {
        requested_target: request.target.clone(),
        normalized_target: entry.normalized_target.clone(),
        resolved_ip: entry.resolved_ip.clone(),
        class: request.class,
        kind: entry.kind,
        observed_at_unix_ms: entry.observed_at_unix_ms,
        reachable: entry.reachable,
        rtt_ms: entry.rtt_ms,
        error: entry.error.clone(),
    }
}

fn failed_observation(
    request: &ProbeRequest,
    normalized_target: &str,
    resolved_ip: Option<String>,
    observed_at_unix_ms: u64,
    error: String,
) -> ProbeObservation {
    ProbeObservation {
        requested_target: request.target.clone(),
        normalized_target: normalized_target.to_string(),
        resolved_ip,
        class: request.class,
        kind: request.kind,
        observed_at_unix_ms,
        reachable: false,
        rtt_ms: None,
        error: Some(error),
    }
}

async fn execute_probe(request: &ProbeRequest, normalized_target: &str) -> ProbeCacheEntry {
    let observed_at_unix_ms = now_unix_ms();
    let resolved_ip = match resolve_target(normalized_target).await {
        Ok(resolved_ip) => resolved_ip,
        Err(error) => {
            return ProbeCacheEntry {
                normalized_target: normalized_target.to_string(),
                resolved_ip: None,
                kind: request.kind,
                observed_at_unix_ms,
                reachable: false,
                rtt_ms: None,
                error: Some(error),
            };
        }
    };
    let (ip, resolved_ip_text) = resolved_ip;

    let client = match ip {
        IpAddr::V4(_) => Client::new(&PingConfig::default()),
        IpAddr::V6(_) => Client::new(&PingConfig::builder().kind(ICMP::V6).build()),
    };
    let Ok(client) = client else {
        return ProbeCacheEntry {
            normalized_target: normalized_target.to_string(),
            resolved_ip: Some(resolved_ip_text),
            kind: request.kind,
            observed_at_unix_ms,
            reachable: false,
            rtt_ms: None,
            error: Some("unable to create ICMP client".to_string()),
        };
    };

    let payload = [0_u8; 56];
    let mut pinger = client.pinger(ip, PingIdentifier(random())).await;
    pinger.timeout(request.timeout);
    match pinger.ping(PingSequence(0), &payload).await {
        Ok((IcmpPacket::V4(..), duration)) | Ok((IcmpPacket::V6(..), duration)) => {
            // Herbert, ping hook goes here. This is the point where the shared probe actor
            // derives the final live result for a probe, so Insight/LTS2 can observe failover
            // timing or emit alerts such as "you are now on backup lothlorien to Rohan."
            ProbeCacheEntry {
                normalized_target: normalized_target.to_string(),
                resolved_ip: Some(resolved_ip_text),
                kind: request.kind,
                observed_at_unix_ms,
                reachable: true,
                rtt_ms: Some(duration.as_secs_f64() * 1000.0),
                error: None,
            }
        }
        Err(err) => ProbeCacheEntry {
            normalized_target: normalized_target.to_string(),
            resolved_ip: Some(resolved_ip_text),
            kind: request.kind,
            observed_at_unix_ms,
            reachable: false,
            rtt_ms: None,
            error: Some(err.to_string()),
        },
    }
}

async fn resolve_target(normalized_target: &str) -> Result<(IpAddr, String), String> {
    if let Ok(ip) = normalized_target.parse::<IpAddr>() {
        return Ok((ip, ip.to_string()));
    }

    let mut addrs = tokio::net::lookup_host((normalized_target, 80))
        .await
        .map_err(|err| format!("unable to resolve target '{normalized_target}': {err}"))?;
    let Some(addr) = addrs.next() else {
        return Err(format!("unable to resolve target '{normalized_target}'"));
    };
    Ok((addr.ip(), addr.ip().to_string()))
}

#[cfg(test)]
mod tests {
    use super::{ProbeKind, ProbeRequest, duration_to_millis, normalize_target};
    use std::time::Duration;

    #[test]
    fn normalize_target_trims_and_canonicalizes() {
        assert_eq!(
            normalize_target(" 100.126.0.1 "),
            Some("100.126.0.1".to_string())
        );
        assert_eq!(
            normalize_target(" Example.COM "),
            Some("example.com".to_string())
        );
        assert_eq!(normalize_target(""), None);
    }

    #[test]
    fn duration_to_millis_saturates_reasonably() {
        assert_eq!(duration_to_millis(Duration::from_millis(250)), 250);
    }

    #[test]
    fn request_builders_set_expected_kind() {
        let reachability = ProbeRequest::reachability(
            "1.1.1.1",
            super::ProbeClass::UiMonitor,
            Duration::from_secs(1),
        );
        let rtt = ProbeRequest::round_trip_time(
            "1.1.1.1",
            super::ProbeClass::Stormguard,
            Duration::from_secs(1),
        );
        assert_eq!(reachability.kind, ProbeKind::Reachability);
        assert_eq!(rtt.kind, ProbeKind::RoundTripTime);
    }
}
