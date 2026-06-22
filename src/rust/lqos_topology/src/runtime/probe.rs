use anyhow::Result;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use lqos_probe::{ProbeClass, ProbeRequest};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::oneshot::error::TryRecvError;

use crate::AttachmentProbeSpec;

use super::TopologyBusSender;

const TOPOLOGY_PROBE_MAX_AGE_MS: u64 = 250;
const BUS_CALL_TIMEOUT_MARGIN: Duration = Duration::from_millis(500);
const BUS_CALL_POLL_INTERVAL: Duration = Duration::from_millis(10);

fn sleep_until_deadline(deadline: Instant) -> bool {
    let now = Instant::now();
    if now >= deadline {
        return false;
    }
    std::thread::sleep(
        deadline
            .saturating_duration_since(now)
            .min(BUS_CALL_POLL_INTERVAL),
    );
    true
}

fn bus_call(
    bus_tx: TopologyBusSender,
    request: BusRequest,
    timeout: Duration,
) -> anyhow::Result<BusReply> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow::anyhow!("Invalid bus call timeout: {timeout:?}"))?;
    let (once_tx, mut once_rx) = tokio::sync::oneshot::channel();
    let request_copy = request.clone();
    let mut pending = Some((once_tx, request));

    while let Some(message) = pending.take() {
        match bus_tx.try_send(message) {
            Ok(()) => break,
            Err(TrySendError::Full(returned)) => {
                pending = Some(returned);
                if !sleep_until_deadline(deadline) {
                    anyhow::bail!("Timed out sending bus request: {request_copy:?}");
                }
            }
            Err(TrySendError::Closed(_)) => {
                anyhow::bail!("Bus closed while sending request: {request_copy:?}");
            }
        }
    }

    loop {
        match once_rx.try_recv() {
            Ok(reply) => return Ok(reply),
            Err(TryRecvError::Empty) => {
                if !sleep_until_deadline(deadline) {
                    anyhow::bail!("Timed out waiting for bus reply: {request_copy:?}");
                }
            }
            Err(TryRecvError::Closed) => {
                anyhow::bail!("Bus reply channel closed for request: {request_copy:?}");
            }
        }
    }
}

pub(super) fn parse_probe_ip(raw: &str) -> Option<IpAddr> {
    raw.trim()
        .split('/')
        .next()
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<IpAddr>().ok())
}

pub(super) fn probe_specs(
    bus_tx: TopologyBusSender,
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

    let response = bus_call(
        bus_tx.clone(),
        BusRequest::ProbeBatch {
            requests: probe_requests,
            max_age_ms: TOPOLOGY_PROBE_MAX_AGE_MS,
        },
        timeout.saturating_add(BUS_CALL_TIMEOUT_MARGIN),
    )
    .map_err(|err| anyhow::anyhow!("unable to query shared probe manager: {err}"))?
    .responses
    .into_iter()
    .next();

    let mut results = HashMap::<String, (bool, bool)>::new();
    match response {
        Some(BusResponse::ProbeObservations(observations)) => {
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
        Some(BusResponse::Fail(message)) => Err(anyhow::anyhow!(
            "shared probe manager rejected topology batch: {message}"
        )),
        other => Err(anyhow::anyhow!(
            "unexpected response from shared probe manager: {other:?}"
        )),
    }
}
