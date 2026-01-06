use crate::node_manager::ws::messages::{FlowbeeKeyTransit, WsResponse, encode_ws_message};
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::throughput_tracker::flow_data::{
    ALL_FLOWS, FlowAnalysis, FlowbeeLocalData, get_asn_name_and_country,
};
use lqos_utils::unix_time::time_since_boot;
use std::net::IpAddr;
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::debug;

const FIVE_MINUTES_AS_NANOS: u64 = 300 * 1_000_000_000;

fn recent_flows_by_circuit(
    circuit_id: &str,
) -> Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)> {
    let device_reader = SHAPED_DEVICES.load();
    if let Ok(now) = time_since_boot() {
        let now_as_nanos = Duration::from(now).as_nanos() as u64;
        let five_minutes_ago = now_as_nanos.saturating_sub(FIVE_MINUTES_AS_NANOS);

        {
            let all_flows = ALL_FLOWS.lock();
            let result: Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)> = all_flows
                .flow_data
                .iter()
                .filter_map(|(key, (local, analysis))| {
                    // Don't show older flows
                    if local.last_seen < five_minutes_ago {
                        return None;
                    }

                    // Don't show flows that don't belong to the circuit
                    let local_ip_str; // Using late binding
                    let remote_ip_str;
                    let device_name;
                    let asn_name;
                    let asn_country;
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
                        let geo = get_asn_name_and_country(key.remote_ip.as_ip());
                        (asn_name, asn_country) = (geo.name, geo.country);
                    } else if let Some(device) = device_reader.trie.longest_match(remote_ip) {
                        // The reverse way around
                        local_ip_str = key.remote_ip.to_string();
                        remote_ip_str = key.local_ip.to_string();
                        let device = &device_reader.devices[*device.1];
                        if device.circuit_id != circuit_id {
                            return None;
                        }
                        device_name = device.device_name.clone();
                        let geo = get_asn_name_and_country(key.local_ip.as_ip());
                        (asn_name, asn_country) = (geo.name, geo.country);
                    } else {
                        return None;
                    }

                    Some((
                        FlowbeeKeyTransit {
                            remote_ip: remote_ip_str,
                            local_ip: local_ip_str,
                            src_port: key.src_port,
                            dst_port: key.dst_port,
                            ip_protocol: key.ip_protocol,
                            device_name,
                            asn_name,
                            asn_country,
                            protocol_name: analysis.protocol_analysis.to_string(),
                            last_seen_nanos: now_as_nanos.saturating_sub(local.last_seen),
                        },
                        local.clone(),
                        analysis.clone(),
                    ))
                })
                .collect();
            return result;
        }
    }
    Vec::new()
}

pub(super) async fn flows_by_circuit(
    circuit: String,
    tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        let flows: Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)> =
            recent_flows_by_circuit(&circuit)
                .into_iter()
                .map(|(key, local, analysis)| (key.into(), local, analysis))
                .collect();

        if !flows.is_empty() {
            let result = WsResponse::FlowsByCircuit {
                circuit_id: circuit.clone(),
                flows,
            };
            if let Ok(payload) = encode_ws_message(&result) {
                if let Err(_) = tx.send(payload).await {
                    debug!("Channel is gone");
                    break;
                }
            } else {
                break;
            }
        }

        ticker.tick().await;
    }
}
