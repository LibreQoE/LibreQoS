mod cadence;
mod throughput;
mod rtt_histogram;
mod flow_counter;
mod top_10;
mod ipstats_conversion;
mod top_flows;
mod flow_endpoints;
pub mod system_info;
mod tree_summary;
mod queue_stats_total;
mod network_tree;

use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use crate::node_manager::ws::publish_subscribe::PubSub;

/// Runs a periodic tick to feed data to the node manager.
pub(super) async fn channel_ticker(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second

        let _ = tokio::join!(
            timeout(Duration::from_secs_f32(0.9), cadence::cadence(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), throughput::throughput(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), rtt_histogram::rtt_histo(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), flow_counter::flow_count(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), top_10::top_10_downloaders(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), top_10::worst_10_downloaders(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), top_10::worst_10_retransmit(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), top_flows::top_flows_bytes(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), top_flows::top_flows_rate(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), flow_endpoints::endpoints_by_country(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), flow_endpoints::ether_protocols(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), flow_endpoints::ip_protocols(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), system_info::cpu_info(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), system_info::ram_info(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), tree_summary::tree_summary(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), queue_stats_total::queue_stats_totals(channels.clone())),
            timeout(Duration::from_secs_f32(0.9), network_tree::network_tree(channels.clone())),
        );
    }
}