use crate::node_manager::ws::messages::{FlowbeeKeyTransit, WsResponse, encode_ws_message};
use crate::shaped_devices_tracker::{SHAPED_DEVICE_HASH_CACHE, SHAPED_DEVICES};
use crate::throughput_tracker::flow_data::{
    ALL_FLOWS, FlowAnalysis, FlowbeeLocalData, get_asn_name_and_country,
};
use lqos_utils::units::DownUpOrder;
use lqos_utils::hash_to_i64;
use lqos_utils::unix_time::time_since_boot;
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::debug;

const RECENT_CIRCUIT_FLOWS_WINDOW_NANOS: u64 = 30 * 1_000_000_000;
const FLOW_RATE_SANITY_MULTIPLIER: f64 = 2.0;
const FLOW_RATE_SANITY_FLOOR_BPS: u64 = 25_000_000;

fn circuit_display_rate_ceiling_bps(
    shaped: &lqos_config::ConfigShapedDevices,
    circuit_hash: i64,
) -> Option<DownUpOrder<u32>> {
    let mut max_down_mbps = 0.0_f32;
    let mut max_up_mbps = 0.0_f32;

    for device in &shaped.devices {
        if device.circuit_hash != circuit_hash {
            continue;
        }
        max_down_mbps = max_down_mbps.max(device.download_max_mbps);
        max_up_mbps = max_up_mbps.max(device.upload_max_mbps);
    }

    if max_down_mbps <= 0.0 && max_up_mbps <= 0.0 {
        return None;
    }

    Some(DownUpOrder {
        down: sanitized_plan_ceiling_bps(max_down_mbps),
        up: sanitized_plan_ceiling_bps(max_up_mbps),
    })
}

const fn clamp_u64_to_u32(value: u64) -> u32 {
    if value > u32::MAX as u64 {
        u32::MAX
    } else {
        value as u32
    }
}

fn sanitized_plan_ceiling_bps(plan_mbps: f32) -> u32 {
    if !plan_mbps.is_finite() || plan_mbps <= 0.0 {
        return clamp_u64_to_u32(FLOW_RATE_SANITY_FLOOR_BPS);
    }

    let scaled = (plan_mbps as f64 * 1_000_000.0 * FLOW_RATE_SANITY_MULTIPLIER).round();
    let scaled = scaled.max(FLOW_RATE_SANITY_FLOOR_BPS as f64) as u64;
    clamp_u64_to_u32(scaled)
}

fn display_rate_bps(
    local: &FlowbeeLocalData,
    ceiling_bps: Option<DownUpOrder<u32>>,
) -> DownUpOrder<u32> {
    let Some(ceiling_bps) = ceiling_bps else {
        return local.rate_estimate_bps;
    };

    DownUpOrder {
        down: local.rate_estimate_bps.down.min(ceiling_bps.down),
        up: local.rate_estimate_bps.up.min(ceiling_bps.up),
    }
}

fn recent_flows_by_circuit(
    circuit_id: &str,
) -> Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysis)> {
    let circuit_hash = hash_to_i64(circuit_id);
    let shaped = SHAPED_DEVICES.load();
    let cache = SHAPED_DEVICE_HASH_CACHE.load();
    let display_rate_ceiling = circuit_display_rate_ceiling_bps(&shaped, circuit_hash);
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

                    let mut local = local.clone();
                    local.set_display_rate_bps(Some(display_rate_bps(&local, display_rate_ceiling)));

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
                        local,
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

#[cfg(test)]
mod tests {
    use super::{
        DownUpOrder, FlowbeeLocalData, display_rate_bps, sanitized_plan_ceiling_bps,
    };

    #[test]
    fn sanitized_plan_ceiling_uses_multiplier_with_floor() {
        assert_eq!(sanitized_plan_ceiling_bps(50.0), 100_000_000);
        assert_eq!(sanitized_plan_ceiling_bps(1.0), 25_000_000);
    }

    #[test]
    fn display_rate_is_clamped_to_circuit_plan_ceiling() {
        let mut local = FlowbeeLocalData {
            start_time: 0,
            last_seen: 0,
            bytes_sent: DownUpOrder::default(),
            packets_sent: DownUpOrder::default(),
            rate_estimate_bps: DownUpOrder::default(),
            display_rate_bps: None,
            tcp_retransmits: DownUpOrder::default(),
            end_status: 0,
            tos: 0,
            tc_handle: 0,
            cpu: 0,
            circuit_hash: None,
            device_hash: None,
            tcp_info: None,
        };
        local.set_rate_estimate_bps(DownUpOrder {
            down: u32::MAX,
            up: 100_000_000,
        });

        let display = display_rate_bps(
            &local,
            Some(DownUpOrder {
                down: 100_000_000,
                up: 25_000_000,
            }),
        );

        assert_eq!(display.down, 100_000_000);
        assert_eq!(display.up, 25_000_000);
    }
}
