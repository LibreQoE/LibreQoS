use std::sync::Arc;

use tokio::spawn;
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

pub use network_tree::{Circuit, all_circuits};

/// Runs a periodic tick to feed data to the node manager.
pub(super) async fn channel_ticker(channels: Arc<PubSub>) {
    spawn(async { one_second_cadence(channels.clone()) });
    spawn(async { two_second_cadence(channels.clone()) });
    spawn(async { five_second_cadence(channels.clone()) });
}

async fn one_second_cadence(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second
        spawn(async { cadence::cadence(channels.clone()) });
        spawn(async { throughput::throughput(channels.clone()) });
        spawn(async { rtt_histogram::rtt_histo(channels.clone()) });
        spawn(async { flow_counter::flow_count(channels.clone()) });
        spawn(async { top_10::top_10_downloaders(channels.clone()) });
        spawn(async { top_10::worst_10_downloaders(channels.clone()) });
        spawn(async { top_10::worst_10_retransmit(channels.clone()) });
        spawn(async { top_flows::top_flows_bytes(channels.clone()) });
        spawn(async { top_flows::top_flows_rate(channels.clone()) });
        spawn(async { flow_endpoints::endpoints_by_country(channels.clone()) });
        spawn(async { flow_endpoints::ether_protocols(channels.clone()) });
        spawn(async { flow_endpoints::ip_protocols(channels.clone()) });
        spawn(async { tree_summary::tree_summary(channels.clone()) });
        spawn(async { network_tree::network_tree(channels.clone()) });
        spawn(async { circuit_capacity::circuit_capacity(channels.clone()) });
        spawn(async { tree_capacity::tree_capacity(channels.clone()) });
    }
}

async fn two_second_cadence(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second
        spawn(async { queue_stats_total::queue_stats_totals(channels.clone()) });
        spawn(async { network_tree::all_subscribers(channels.clone()) });
    }
}

async fn five_second_cadence(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second
        spawn(async { system_info::cpu_info(channels.clone()) });
        spawn(async { system_info::ram_info(channels.clone()) });
    }
}