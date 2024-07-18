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
    let mc = channels.clone(); spawn(async move { one_second_cadence(mc) });
    let mc = channels.clone(); spawn(async move { two_second_cadence(mc) });
    let mc = channels.clone(); spawn(async move { five_second_cadence(mc) });
}

async fn one_second_cadence(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second
        let mc = channels.clone(); spawn(async move { cadence::cadence(mc) });
        let mc = channels.clone(); spawn(async move { throughput::throughput(mc) });
        let mc = channels.clone(); spawn(async move { rtt_histogram::rtt_histo(mc) });
        let mc = channels.clone(); spawn(async move { flow_counter::flow_count(mc) });
        let mc = channels.clone(); spawn(async move { top_10::top_10_downloaders(mc) });
        let mc = channels.clone(); spawn(async move { top_10::worst_10_downloaders(mc) });
        let mc = channels.clone(); spawn(async move { top_10::worst_10_retransmit(mc) });
        let mc = channels.clone(); spawn(async move { top_flows::top_flows_bytes(mc) });
        let mc = channels.clone(); spawn(async move { top_flows::top_flows_rate(mc) });
        let mc = channels.clone(); spawn(async move { flow_endpoints::endpoints_by_country(mc) });
        let mc = channels.clone(); spawn(async move { flow_endpoints::ether_protocols(mc) });
        let mc = channels.clone(); spawn(async move { flow_endpoints::ip_protocols(mc) });
        let mc = channels.clone(); spawn(async move { tree_summary::tree_summary(mc) });
        let mc = channels.clone(); spawn(async move { network_tree::network_tree(mc) });
        let mc = channels.clone(); spawn(async move { circuit_capacity::circuit_capacity(mc) });
        let mc = channels.clone(); spawn(async move { tree_capacity::tree_capacity(mc) });
    }
}

async fn two_second_cadence(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second
        let mc = channels.clone(); spawn(async move { queue_stats_total::queue_stats_totals(mc) });
        let mc = channels.clone(); spawn(async move { network_tree::all_subscribers(mc) });
    }
}

async fn five_second_cadence(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second
        let mc = channels.clone(); spawn(async move { system_info::cpu_info(mc) });
        let mc = channels.clone(); spawn(async move { system_info::ram_info(mc) });
    }
}