use std::sync::Arc;

use tokio::join;
use tracing::debug;
use crate::node_manager::ws::publish_subscribe::PubSub;
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
mod circuit_capacity;
mod tree_capacity;

pub use network_tree::all_circuits;

/// Runs a periodic tick to feed data to the node manager.
pub(super) async fn channel_ticker(
    channels: Arc<PubSub>,
) {
    debug!("Starting channel tickers");
    one_second_cadence(channels.clone()).await;
}

async fn one_second_cadence(
    channels: Arc<PubSub>,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await; // Once per second
        channels.update_living_channel_list().await;
        join!(
            cadence::cadence(channels.clone()),
            throughput::throughput(channels.clone()),
            rtt_histogram::rtt_histo(channels.clone()),
            flow_counter::flow_count(channels.clone()),
            top_10::top_10_downloaders(channels.clone()),
            top_10::worst_10_downloaders(channels.clone()),
            top_10::worst_10_retransmit(channels.clone()),
            top_flows::top_flows_bytes(channels.clone()),
            top_flows::top_flows_rate(channels.clone()),
            flow_endpoints::endpoints_by_country(channels.clone()),
            flow_endpoints::ether_protocols(channels.clone()),
            flow_endpoints::ip_protocols(channels.clone()),
            flow_endpoints::flow_duration(channels.clone()),
            tree_summary::tree_summary(channels.clone()),
            network_tree::network_tree(channels.clone()),
            circuit_capacity::circuit_capacity(channels.clone()),
            tree_capacity::tree_capacity(channels.clone()),
            system_info::cpu_info(channels.clone()),
            system_info::ram_info(channels.clone()),
            queue_stats_total::queue_stats_totals(channels.clone()),
            network_tree::all_subscribers(channels.clone()),
        );

        channels.clean().await;
    }
}