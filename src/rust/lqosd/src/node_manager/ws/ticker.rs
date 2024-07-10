use std::sync::Arc;
use std::time::Duration;

use tokio::join;
use tokio::time::timeout;

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

/// Runs a periodic tick to feed data to the node manager.
pub(super) async fn channel_ticker(channels: Arc<PubSub>) {
    join!(
        one_second_cadence(channels.clone()),
        two_second_cadence(channels.clone()),
        five_second_cadence(channels.clone()),
    );
}

async fn one_second_cadence(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    let timeout_time = Duration::from_secs_f32(0.9);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second
        let _ = join!(
            timeout(timeout_time, cadence::cadence(channels.clone())),
            timeout(timeout_time, throughput::throughput(channels.clone())),
            timeout(timeout_time, rtt_histogram::rtt_histo(channels.clone())),
            timeout(timeout_time, flow_counter::flow_count(channels.clone())),
            timeout(timeout_time, top_10::top_10_downloaders(channels.clone())),
            timeout(timeout_time, top_10::worst_10_downloaders(channels.clone())),
            timeout(timeout_time, top_10::worst_10_retransmit(channels.clone())),
            timeout(timeout_time, top_flows::top_flows_bytes(channels.clone())),
            timeout(timeout_time, top_flows::top_flows_rate(channels.clone())),
            timeout(timeout_time, flow_endpoints::endpoints_by_country(channels.clone())),
            timeout(timeout_time, flow_endpoints::ether_protocols(channels.clone())),
            timeout(timeout_time, flow_endpoints::ip_protocols(channels.clone())),
            timeout(timeout_time, tree_summary::tree_summary(channels.clone())),
            timeout(timeout_time, network_tree::network_tree(channels.clone())),
        );
    }
}

async fn two_second_cadence(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
    let timeout_time = Duration::from_secs_f32(1.9);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second
        let _ = join!(
            timeout(timeout_time, queue_stats_total::queue_stats_totals(channels.clone())),
            timeout(timeout_time, network_tree::all_subscribers(channels.clone())),
        );
    }
}

async fn five_second_cadence(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
    let timeout_time = Duration::from_secs_f32(4.9);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second
        let _ = join!(
            timeout(timeout_time, system_info::cpu_info(channels.clone())),
            timeout(timeout_time, system_info::ram_info(channels.clone())),
       );
    }
}