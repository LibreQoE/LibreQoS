use std::sync::Arc;

use tokio::join;
use tokio::sync::mpsc::Sender;
use tracing::debug;
use lqos_bus::BusRequest;
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
use crate::system_stats::SystemStats;

/// Runs a periodic tick to feed data to the node manager.
pub(super) async fn channel_ticker(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>
) {
    debug!("Starting channel tickers");
    one_second_cadence(channels.clone(), bus_tx.clone(), system_usage_tx.clone()).await;
}

async fn one_second_cadence(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await; // Once per second
        channels.update_living_channel_list().await;
        join!(
            cadence::cadence(channels.clone()),
            throughput::throughput(channels.clone(), bus_tx.clone()),
            rtt_histogram::rtt_histo(channels.clone(), bus_tx.clone()),
            flow_counter::flow_count(channels.clone(), bus_tx.clone()),
            top_10::top_10_downloaders(channels.clone(), bus_tx.clone()),
            top_10::worst_10_downloaders(channels.clone(), bus_tx.clone()),
            top_10::worst_10_retransmit(channels.clone(), bus_tx.clone()),
            top_flows::top_flows_bytes(channels.clone(), bus_tx.clone()),
            top_flows::top_flows_rate(channels.clone(), bus_tx.clone()),
            flow_endpoints::endpoints_by_country(channels.clone(), bus_tx.clone()),
            flow_endpoints::ether_protocols(channels.clone(), bus_tx.clone()),
            flow_endpoints::ip_protocols(channels.clone(), bus_tx.clone()),
            flow_endpoints::flow_duration(channels.clone(), bus_tx.clone()),
            tree_summary::tree_summary(channels.clone(), bus_tx.clone()),
            network_tree::network_tree(channels.clone(), bus_tx.clone()),
            circuit_capacity::circuit_capacity(channels.clone()),
            tree_capacity::tree_capacity(channels.clone()),
            system_info::cpu_info(channels.clone(), system_usage_tx.clone()),
            system_info::ram_info(channels.clone(), system_usage_tx.clone()),
            queue_stats_total::queue_stats_totals(channels.clone()),
            network_tree::all_subscribers(channels.clone(), bus_tx.clone()),

        );

        channels.clean().await;
    }
}