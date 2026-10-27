use lqos_utils::XdpIpAddress;
use std::net::IpAddr;

/// K1 §1.6: the store file schema lives in `lqos_config` (the shared low-level
/// crate) so `lqos_topology` can fold it without depending on this crate.
/// Re-exported here so existing consumers keep their path.
pub use lqos_config::DynamicCircuit;

/// Minimal kernel observation used to drive dynamic-circuit activity updates.
///
/// Hash fields come from the kernel IP map; when present they should be preferred over
/// any trie-derived lookups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CircuitObservation {
    pub ip: XdpIpAddress,
    pub device_hash: Option<i64>,
    pub circuit_hash: Option<i64>,
}

pub(crate) fn is_superseded_by_shaped_devices(
    circuit: &DynamicCircuit,
    shaped_devices: &crate::ShapedDevicesCatalog,
) -> bool {
    if shaped_devices
        .device_by_hashes(
            Some(circuit.shaped.device_hash),
            Some(circuit.shaped.circuit_hash),
        )
        .is_some()
    {
        return true;
    }

    for (ip, _) in circuit.shaped.ipv4.iter() {
        let probe = XdpIpAddress::from_ip(IpAddr::V4(*ip));
        if shaped_devices.device_longest_match_for_ip(&probe).is_some() {
            return true;
        }
    }
    for (ip, _) in circuit.shaped.ipv6.iter() {
        let probe = XdpIpAddress::from_ip(IpAddr::V6(*ip));
        if shaped_devices.device_longest_match_for_ip(&probe).is_some() {
            return true;
        }
    }

    false
}

pub(crate) fn expired_dynamic_circuit_ids(
    circuits: &[DynamicCircuit],
    now_unix: u64,
    ttl_seconds: u64,
) -> Vec<String> {
    circuits
        .iter()
        .filter_map(|circuit| {
            let circuit_id = circuit.shaped.circuit_id.trim();
            if circuit_id.is_empty() {
                return None;
            }
            let age_seconds = now_unix.saturating_sub(circuit.last_seen_unix);
            (age_seconds > ttl_seconds).then_some(circuit_id.to_string())
        })
        .collect()
}
