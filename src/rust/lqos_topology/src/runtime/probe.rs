use anyhow::Result;
use lqos_bus::{BusReply, BusRequest, BusResponse};
use lqos_probe::{ProbeClass, ProbeRequest};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use crate::AttachmentProbeSpec;

use super::TopologyBusSender;

const TOPOLOGY_PROBE_MAX_AGE_MS: u64 = 250;

fn bus_call(bus_tx: TopologyBusSender, request: BusRequest) -> anyhow::Result<BusReply> {
    let (once_tx, once_rx) = tokio::sync::oneshot::channel();
    let request_copy = request.clone();
    bus_tx.blocking_send((once_tx, request))?;
    if let Ok(reply) = once_rx.blocking_recv() {
        Ok(reply)
    } else {
        anyhow::bail!("Call to {:?} failed", request_copy);
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
