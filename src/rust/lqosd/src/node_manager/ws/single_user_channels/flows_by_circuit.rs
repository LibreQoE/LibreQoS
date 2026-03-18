use crate::node_manager::ws::messages::{FlowbeeKeyTransit, WsResponse, encode_ws_message};
use crate::shaped_devices_tracker::{SHAPED_DEVICE_HASH_CACHE, SHAPED_DEVICES};
use crate::throughput_tracker::flow_data::{
    ALL_FLOWS, FlowAnalysis, FlowbeeLocalData, get_asn_name_and_country,
};
use lqos_utils::hash_to_i64;
use lqos_utils::unix_time::time_since_boot;
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::debug;

const RECENT_CIRCUIT_FLOWS_WINDOW_NANOS: u64 = 30 * 1_000_000_000;

fn recent_flows_by_circuit(
    circuit_id: &str,
) -> Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)> {
    let circuit_hash = hash_to_i64(circuit_id);
    let shaped = SHAPED_DEVICES.load();
    let cache = SHAPED_DEVICE_HASH_CACHE.load();
    if let Ok(now) = time_since_boot() {
        let now_as_nanos = Duration::from(now).as_nanos() as u64;
        let recent_cutoff = now_as_nanos.saturating_sub(RECENT_CIRCUIT_FLOWS_WINDOW_NANOS);

        {
            let all_flows = ALL_FLOWS.lock();
            let result: Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)> = all_flows
                .flow_data
                .iter()
                .filter_map(|(key, (local, analysis))| {
                    // Don't show older flows
                    if local.last_seen < recent_cutoff {
                        return None;
                    }

                    if local.circuit_hash != Some(circuit_hash) {
                        return None;
                    }

                    let device_name = local
                        .device_hash
                        .and_then(|hash| cache.index_by_device_hash(&shaped, hash))
                        .and_then(|idx| shaped.devices.get(idx))
                        .map(|d| d.device_name.clone())
                        .unwrap_or_else(|| "Unknown".to_string());

                    let geo = get_asn_name_and_country(key.remote_ip.as_ip());
                    let (local_ip_str, remote_ip_str, asn_name, asn_country) = (
                        key.local_ip.to_string(),
                        key.remote_ip.to_string(),
                        geo.name,
                        geo.country,
                    );

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
                        *analysis,
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
            recent_flows_by_circuit(&circuit).into_iter().collect();

        let result = WsResponse::FlowsByCircuit {
            circuit_id: circuit.clone(),
            flows,
        };
        if let Ok(payload) = encode_ws_message(&result) {
            if tx.send(payload).await.is_err() {
                debug!("Channel is gone");
                break;
            }
        } else {
            break;
        }

        ticker.tick().await;
    }
}
