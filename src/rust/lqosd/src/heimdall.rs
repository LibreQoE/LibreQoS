//! Heimdall watch-session resolution and scoping
//! (`docs/heimdall_bridge_rederivation.md` §1.4a + §1.5).
//!
//! The bridge's watch set is bridge leaf SLOT ids (its hot-path gate compares
//! `metadata.circuit_id`); the product boundary is IP-based. This module owns
//! the IP → circuit → slot join (the network-devices catalog + the bridge
//! analytics snapshot) and the F3 scoping check, then dispatches to the
//! transport selected at startup. `lqos_heimdall` can see neither the catalog
//! nor the bridge slot table, so the resolution lives here.

use std::net::IpAddr;

use lqos_heimdall::{HeimdallTransport, hyperfocus_on_target_bridge};
use lqos_utils::XdpIpAddress;

use crate::bridge_analytics;
use crate::node_manager::LoginResult;

/// Resolves the circuit a subscriber IP belongs to, via the catalog's
/// longest-prefix match. `None` = the IP is in no known circuit.
fn circuit_name_for_ip(ip: IpAddr) -> Option<String> {
    let catalog = lqos_network_devices::network_devices_catalog();
    let (_, device) = catalog.device_longest_match_for_ip(&XdpIpAddress::from(ip))?;
    let name = device.circuit_id.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// The bridge slot id for a resolved circuit name, from the analytics
/// snapshot's per-circuit `bridge_circuit_id`. `None` = the bridge has not
/// seen the circuit yet (no slot to watch).
fn bridge_slot_for_circuit(circuit_name: &str) -> Option<u64> {
    let snapshot = bridge_analytics::snapshot();
    snapshot
        .by_circuit_id
        .get(circuit_name)
        .map(|row| row.bridge_circuit_id)
}

/// F3 (§1.5): starts a packet-capture session for `ip`. Admin-gated; the IP
/// must belong to a known circuit; the transport is the one selected at
/// startup (§1.4a). Returns `(session_id, countdown)` on success.
pub fn start_analysis(login: LoginResult, ip: IpAddr) -> Option<(usize, usize)> {
    if login != LoginResult::Admin {
        tracing::warn!("Rejected packet-capture request for {ip}: administrator login required");
        return None;
    }
    let Some(circuit_name) = circuit_name_for_ip(ip) else {
        tracing::warn!("Rejected packet-capture request for {ip}: no known circuit");
        return None;
    };
    match lqos_heimdall::transport()? {
        HeimdallTransport::Bridge { socket } => {
            let Some(slot_id) = bridge_slot_for_circuit(&circuit_name) else {
                tracing::warn!(
                    "Rejected packet-capture request for {ip}: circuit {circuit_name} has no bridge slot yet"
                );
                return None;
            };
            hyperfocus_on_target_bridge(slot_id, ip, socket)
        }
    }
}

/// The privileged (in-process bus) entry point: the local bus is root-only,
/// so the admin gate is satisfied by construction.
pub fn start_analysis_privileged(ip: IpAddr) -> Option<(usize, usize)> {
    start_analysis(LoginResult::Admin, ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F3 §1.5: a non-admin login never starts a capture session.
    #[test]
    fn non_admin_capture_requests_are_rejected() {
        let ip: IpAddr = "192.0.2.1".parse().expect("test invariant");
        assert!(start_analysis(LoginResult::ReadOnly, ip).is_none());
        assert!(start_analysis(LoginResult::Denied, ip).is_none());
    }

    /// F3 §1.5: an IP in no known circuit is rejected (the existing
    /// watch-nothing behavior). 192.0.2.0/24 is TEST-NET-1 — never in a
    /// loaded catalog, and the unit-test catalog is empty.
    #[test]
    fn out_of_circuit_capture_requests_are_rejected() {
        let ip: IpAddr = "192.0.2.1".parse().expect("test invariant");
        assert!(start_analysis(LoginResult::Admin, ip).is_none());
    }
}
