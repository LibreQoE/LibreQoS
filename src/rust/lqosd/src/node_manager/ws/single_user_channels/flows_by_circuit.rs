use std::net::IpAddr;
use std::time::Duration;
use serde::Serialize;
use tokio::time::MissedTickBehavior;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::unix_time::time_since_boot;
use lqos_utils::XdpIpAddress;
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::throughput_tracker::flow_data::{ALL_FLOWS, FlowAnalysis, FlowbeeLocalData, get_asn_name_and_country};

const FIVE_MINUTES_AS_NANOS: u64 = 300 * 1_000_000_000;

fn recent_flows_by_circuit(circuit_id: &str) -> Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)> {
    let device_reader = SHAPED_DEVICES.read().unwrap();

    if let Ok(now) = time_since_boot() {
        let now_as_nanos = Duration::from(now).as_nanos() as u64;
        let five_minutes_ago = now_as_nanos - FIVE_MINUTES_AS_NANOS;

        let all_flows = ALL_FLOWS.lock().unwrap();
        let result: Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)> = all_flows
            .iter()
            .filter_map(|(key, (local, analysis))| {
                // Don't show older flows
                if local.last_seen < five_minutes_ago {
                    return None;
                }

                // Don't show flows that don't belong to the circuit
                let mut local_ip_str = String::new();
                let mut remote_ip_str = String::new();
                let mut device_name = String::new();
                let mut asn_name = String::new();
                let mut asn_country = String::new();
                let local_ip = match key.local_ip.as_ip() {
                    IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                    IpAddr::V6(ip) => ip,
                };
                let remote_ip = match key.remote_ip.as_ip() {
                    IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                    IpAddr::V6(ip) => ip,
                };
                if let Some(device) = device_reader.trie.longest_match(local_ip) {
                    // The normal way around
                    local_ip_str = key.local_ip.to_string();
                    remote_ip_str = key.remote_ip.to_string();
                    let device = &device_reader.devices[*device.1];
                    if device.circuit_id != circuit_id {
                        return None;
                    }
                    device_name = device.device_name.clone();
                    (asn_name, asn_country) = get_asn_name_and_country(key.remote_ip.as_ip());
                } else if let Some(device) = device_reader.trie.longest_match(remote_ip) {
                    // The reverse way around
                    local_ip_str = key.remote_ip.to_string();
                    remote_ip_str = key.local_ip.to_string();
                    let device = &device_reader.devices[*device.1];
                    if device.circuit_id != circuit_id {
                        return None;
                    }
                    device_name = device.device_name.clone();
                    (asn_name, asn_country) = get_asn_name_and_country(key.local_ip.as_ip());
                } else {
                    return None;
                }

                Some((FlowbeeKeyTransit {
                    remote_ip: remote_ip_str,
                    local_ip: local_ip_str,
                    src_port: key.src_port,
                    dst_port: key.dst_port,
                    ip_protocol: key.ip_protocol,
                    device_name,
                    asn_name,
                    asn_country,
                    protocol_name: analysis.protocol_analysis.to_string(),
                }, local.clone(), analysis.clone()))
            })
            .collect();
        return result;
    }
    Vec::new()
}

#[derive(Serialize)]
pub struct FlowbeeKeyTransit {
    /// Mapped `XdpIpAddress` source for the flow.
    pub remote_ip: String,
    /// Mapped `XdpIpAddress` destination for the flow
    pub local_ip: String,
    /// Source port number, or ICMP type.
    pub src_port: u16,
    /// Destination port number.
    pub dst_port: u16,
    /// IP protocol (see the Linux kernel!)
    pub ip_protocol: u8,
    /// Device Name
    pub device_name: String,
    /// ASN Name
    pub asn_name: String,
    /// ASN Country
    pub asn_country: String,
    /// Protocol Name
    pub protocol_name: String,
}

#[derive(Serialize)]
struct FlowData {
    circuit_id: String,
    flows: Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)>,
}

pub(super) async fn flows_by_circuit(circuit: String, tx: tokio::sync::mpsc::Sender<String>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        let flows: Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)> = recent_flows_by_circuit(&circuit)
            .into_iter()
            .map(|(key, local, analysis)| {
                (key.into(), local, analysis)
            })
            .collect();

        if !flows.is_empty() {
            let result = FlowData {
                circuit_id: circuit.clone(),
                flows,
            };
            let message = serde_json::to_string(&result).unwrap();
            if let Err(_) = tx.send(message).await {
                log::info!("Channel is gone");
                break;
            }
        }

        ticker.tick().await;
    }
}