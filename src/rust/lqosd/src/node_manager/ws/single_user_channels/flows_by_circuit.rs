use std::net::IpAddr;
use std::time::Duration;
use serde::Serialize;
use tokio::time::MissedTickBehavior;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::unix_time::time_since_boot;
use lqos_utils::XdpIpAddress;
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::throughput_tracker::flow_data::{ALL_FLOWS, FlowAnalysis, FlowbeeLocalData};

const FIVE_MINUTES_AS_NANOS: u64 = 300 * 1_000_000_000;

fn recent_flows_by_circuit(circuit_id: &str) -> Vec<(FlowbeeKey, FlowbeeLocalData, FlowAnalysis)> {
    let device_reader = SHAPED_DEVICES.read().unwrap();

    if let Ok(now) = time_since_boot() {
        let now_as_nanos = Duration::from(now).as_nanos() as u64;
        let five_minutes_ago = now_as_nanos - FIVE_MINUTES_AS_NANOS;

        let all_flows = ALL_FLOWS.lock().unwrap();
        let result: Vec<(FlowbeeKey, FlowbeeLocalData, FlowAnalysis)> = all_flows
            .iter()
            .filter(|(key, (local, analysis))| {
                local.last_seen > five_minutes_ago
            })
            .filter(|(key, (local, analysis))| {
                let local_ip = match key.local_ip.as_ip() {
                    IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                    IpAddr::V6(ip) => ip,
                };
                let remote_ip = match key.remote_ip.as_ip() {
                    IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                    IpAddr::V6(ip) => ip,
                };
                device_reader.trie.longest_match(local_ip).is_some() || device_reader.trie.longest_match(remote_ip).is_some()
            })
            .map(|(key, (local, analysis))| {
                (key.clone(), local.clone(), analysis.clone())
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
}

impl From<FlowbeeKey> for FlowbeeKeyTransit {
    fn from(key: FlowbeeKey) -> Self {
        FlowbeeKeyTransit {
            remote_ip: key.remote_ip.as_ip().to_string(),
            local_ip: key.local_ip.as_ip().to_string(),
            src_port: key.src_port,
            dst_port: key.dst_port,
            ip_protocol: key.ip_protocol,
        }
    }
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