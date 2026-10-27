use crate::node_manager::ws::messages::{ThroughputData, WsResponse};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use lqos_bus::{BusRequest, BusResponse};
use lqos_config::load_config;
use lqos_utils::units::DownUpOrder;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

use super::request_internal_bus;

pub async fn throughput(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
) {
    if !channels
        .is_channel_alive(PublishedChannels::Throughput)
        .await
    {
        return;
    }

    let request = BusRequest::GetCurrentThroughput;
    let Some(replies) = request_internal_bus("Throughput", bus_tx, request).await else {
        return;
    };
    for reply in replies.responses.into_iter() {
        if let BusResponse::CurrentThroughput {
            bits_per_second,
            packets_per_second,
            tcp_packets_per_second,
            udp_packets_per_second,
            icmp_packets_per_second,
            shaped_bits_per_second,
        } = reply
        {
            let max = if let Ok(config) = load_config() {
                let (down_mbps, up_mbps) = config.bandwidth_mbps();
                // The dashboard scale line is integer Mbps; a decimal root rate
                // rounds here only (the config keeps the exact value).
                DownUpOrder::new(up_mbps.round() as u64, down_mbps.round() as u64)
            } else {
                DownUpOrder::zeroed()
            };

            // `.64.2.2`: on a bridge box `bits_per_second == shaped_bits_per_second`
            // (both the delivered/shaped rate); the offered DEMAND is the bridge's
            // own `bridge_actual_bytes_per_second`. Legacy mode already carries the
            // demand in `bits_per_second`.
            // `.66.2`: the DEMAND class split (mapped/unmapped) comes from the
            // bridge's diffed `.66.1` counters; zeroed in legacy mode.
            let (actual_bps, mapped_bps, unmapped_bps) =
                if crate::bridge_analytics::is_active() {
                    let snapshot = crate::bridge_analytics::snapshot();
                    (
                        snapshot.bridge_actual_bytes_per_second.to_bits_from_bytes(),
                        snapshot.bridge_mapped_bytes_per_second.to_bits_from_bytes(),
                        snapshot.bridge_unmapped_bytes_per_second.to_bits_from_bytes(),
                    )
                } else {
                    (bits_per_second.clone(), DownUpOrder::zeroed(), DownUpOrder::zeroed())
                };

            let unmapped_pct = unmapped_class_pct(&mapped_bps, &unmapped_bps);
            let bps = WsResponse::Throughput {
                data: ThroughputData {
                    bps: bits_per_second.clone(),
                    pps: packets_per_second,
                    tcp_pps: tcp_packets_per_second,
                    udp_pps: udp_packets_per_second,
                    icmp_pps: icmp_packets_per_second,
                    shaped_bps: shaped_bits_per_second,
                    actual_bps: actual_bps.clone(),
                    mapped_bps,
                    unmapped_bps,
                    unmapped_pct,
                    max,
                },
            };
            channels.send(PublishedChannels::Throughput, bps).await;
        }
    }
}


/// `.117`: the pulse's unmapped share is computed from the SAME class counters
/// it displays, so the percentage can never disagree with them. Equal mapped
/// and unmapped volume is 50% (never 100%); zero class volume is 0%.
pub(crate) fn unmapped_class_pct(
    mapped: &DownUpOrder<u64>,
    unmapped: &DownUpOrder<u64>,
) -> f32 {
    let mapped_total = mapped.down.saturating_add(mapped.up);
    let unmapped_total = unmapped.down.saturating_add(unmapped.up);
    let total = mapped_total.saturating_add(unmapped_total);
    if total == 0 {
        0.0
    } else {
        ((unmapped_total as f64 * 100.0) / total as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmapped_pct_uses_the_same_class_counters_as_the_pulse() {
        assert_eq!(
            unmapped_class_pct(&DownUpOrder::new(100, 100), &DownUpOrder::new(100, 100)),
            50.0,
            "equal mapped/unmapped is 50%, never 100%"
        );
        assert_eq!(
            unmapped_class_pct(&DownUpOrder::new(100, 100), &DownUpOrder::new(0, 0)),
            0.0
        );
        assert_eq!(
            unmapped_class_pct(&DownUpOrder::new(0, 0), &DownUpOrder::new(100, 100)),
            100.0
        );
        assert_eq!(
            unmapped_class_pct(&DownUpOrder::new(0, 0), &DownUpOrder::new(0, 0)),
            0.0
        );
    }
}
