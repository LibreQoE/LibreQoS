use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use lqos_bus::{BusRequest, BusResponse, TcHandle, bus_request, IpMapping};
use tokio::runtime::Builder;
use tracing::info;
use std::time::Instant;

use crate::BakeryCommands;

/// Internal representation for desired mapping values per direction.
struct DesiredMaps {
    down: HashMap<String, (TcHandle, u32)>,
    up: HashMap<String, (TcHandle, u32)>,
    down_handles: HashSet<TcHandle>,
    up_handles: HashSet<TcHandle>,
}

/// Builds the desired mapping sets and known handle sets from the circuit definitions.
fn build_desired_maps(
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    on_a_stick: bool,
) -> DesiredMaps {
    let mut down: HashMap<String, (TcHandle, u32)> = HashMap::new();
    let mut up: HashMap<String, (TcHandle, u32)> = HashMap::new();
    let mut down_handles: HashSet<TcHandle> = HashSet::new();
    let mut up_handles: HashSet<TcHandle> = HashSet::new();

    for (_hash, cmd) in circuits.iter() {
        if let BakeryCommands::AddCircuit {
            parent_class_id,
            up_parent_class_id,
            ip_addresses,
            down_cpu,
            up_cpu,
            ..
        } = cmd.as_ref()
        {
            down_handles.insert(*parent_class_id);
            up_handles.insert(*up_parent_class_id);

            if ip_addresses.is_empty() {
                continue;
            }
            for token in ip_addresses.split(',') {
                let ip = token.trim();
                if ip.is_empty() {
                    continue;
                }
                down.insert(ip.to_string(), (*parent_class_id, *down_cpu));
                if on_a_stick {
                    up.insert(ip.to_string(), (*up_parent_class_id, *up_cpu));
                }
            }
        }
    }

    DesiredMaps {
        down,
        up,
        down_handles,
        up_handles,
    }
}

/// Canonicalize current mapping key into the same form as desired tokens
/// For hosts, omit /32 (IPv4) or /128 (IPv6); for subnets, include "/prefix".
fn canonical_key(ip: &str, prefix_len: u32) -> String {
    if let Ok(addr) = ip.parse::<IpAddr>() {
        match addr {
            IpAddr::V4(_) => {
                if prefix_len >= 32 {
                    addr.to_string()
                } else {
                    format!("{}/{}", addr, prefix_len)
                }
            }
            IpAddr::V6(_) => {
                if prefix_len >= 128 {
                    addr.to_string()
                } else {
                    format!("{}/{}", addr, prefix_len)
                }
            }
        }
    } else {
        // Unexpected non-IP string; return as-is with prefix if present
        if prefix_len == 0 {
            ip.to_string()
        } else {
            format!("{}/{}", ip, prefix_len)
        }
    }
}

/// Retrieve the current mappings via the bus and partition into up/down using known handle sets.
fn get_current_maps(
    desired: &DesiredMaps,
) -> Result<(HashMap<String, (TcHandle, u32)>, HashMap<String, (TcHandle, u32)>)> {
    let rt = Builder::new_current_thread().enable_all().build()?;
    let responses = rt.block_on(async { bus_request(vec![BusRequest::ListIpFlow]).await })?;
    let mut down: HashMap<String, (TcHandle, u32)> = HashMap::new();
    let mut up: HashMap<String, (TcHandle, u32)> = HashMap::new();

    for resp in responses {
        match resp {
            BusResponse::MappedIps(list) => {
                for IpMapping { ip_address, prefix_length, tc_handle, cpu } in list {
                    let key = canonical_key(&ip_address, prefix_length);
                    if desired.down_handles.contains(&tc_handle) {
                        down.insert(key, (tc_handle, cpu));
                    } else if desired.up_handles.contains(&tc_handle) {
                        up.insert(key, (tc_handle, cpu));
                    } else {
                        // Unknown mapping; do not touch
                    }
                }
            }
            BusResponse::Fail(e) => return Err(anyhow!("ListIpFlow failed: {}", e)),
            _ => {}
        }
    }
    Ok((down, up))
}

/// Apply the diff via bus: upsert changes then delete stale mappings. Finally clear hot cache.
fn apply_diff(
    desired: &DesiredMaps,
    current_down: &HashMap<String, (TcHandle, u32)>,
    current_up: &HashMap<String, (TcHandle, u32)>,
) -> Result<()> {
    let started = Instant::now();
    // Determine which entries to upsert (add or update)
    let mut upserts: Vec<BusRequest> = Vec::new();
    for (ip, (tc, cpu)) in desired.down.iter() {
        match current_down.get(ip) {
            Some((ctc, ccpu)) if ctc == tc && ccpu == cpu => {}
            _ => upserts.push(BusRequest::MapIpToFlow {
                ip_address: ip.to_string(),
                tc_handle: *tc,
                cpu: *cpu,
                upload: false,
            }),
        }
    }
    for (ip, (tc, cpu)) in desired.up.iter() {
        match current_up.get(ip) {
            Some((ctc, ccpu)) if ctc == tc && ccpu == cpu => {}
            _ => upserts.push(BusRequest::MapIpToFlow {
                ip_address: ip.to_string(),
                tc_handle: *tc,
                cpu: *cpu,
                upload: true,
            }),
        }
    }

    // Determine deletions: present in current but not in desired (directional)
    let mut deletes: Vec<BusRequest> = Vec::new();
    for ip in current_down.keys() {
        if !desired.down.contains_key(ip) {
            deletes.push(BusRequest::DelIpFlow {
                ip_address: ip.to_string(),
                upload: false,
            });
        }
    }
    for ip in current_up.keys() {
        if !desired.up.contains_key(ip) {
            deletes.push(BusRequest::DelIpFlow {
                ip_address: ip.to_string(),
                upload: true,
            });
        }
    }

    let upsert_count = upserts.len();
    let delete_count = deletes.len();
    if upsert_count == 0 && delete_count == 0 {
        return Ok(()); // Nothing to do
    }

    // Batch requests to avoid large payloads; finish with a ClearHotCache
    const CHUNK: usize = 512;
    let mut requests: Vec<BusRequest> = Vec::new();
    requests.extend(upserts.into_iter());
    requests.extend(deletes.into_iter());
    let mut idx = 0;
    let rt = Builder::new_current_thread().enable_all().build()?;
    while idx < requests.len() {
        let end = usize::min(idx + CHUNK, requests.len());
        let mut chunk = requests[idx..end].to_vec();
        // Only add ClearHotCache on the final chunk
        if end == requests.len() {
            chunk.push(BusRequest::ClearHotCache);
        }
        let responses = rt.block_on(async { bus_request(chunk).await })?;
        // Scan for failures
        for resp in responses {
            if let BusResponse::Fail(e) = resp {
                return Err(anyhow!("Mapping update failed: {}", e));
            }
        }
        idx = end;
    }
    let elapsed = started.elapsed();
    info!(
        "Mapping update: upserts={}, deletes={}, elapsed_ms={}",
        upsert_count,
        delete_count,
        elapsed.as_millis()
    );
    Ok(())
}

/// Public entry point: compute desired mappings from circuits, fetch current
/// mappings via bus, diff and apply updates via bus.
pub(crate) fn update_ip_mappings_via_bus(
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    config: &Arc<lqos_config::Config>,
) -> Result<()> {
    let desired = build_desired_maps(circuits, config.on_a_stick_mode());
    let (current_down, current_up) = get_current_maps(&desired)?;
    apply_diff(&desired, &current_down, &current_up)
}
