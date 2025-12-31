use std::sync::Arc;

use crate::node_manager::ws::publish_subscribe::PubSub;
use lqos_bus::BusRequest;
use tokio::join;
use tokio::sync::mpsc::Sender;
use tracing::debug;
mod asn_top;
mod bakery;
mod cadence;
mod circuit_capacity;
mod endpoint_latlon;
mod executive_heatmaps;
mod flow_counter;
mod flow_endpoints;
pub(crate) mod ipstats_conversion;
mod network_tree;
mod queue_stats_total;
mod retransmits;
mod rtt_histogram;
mod stormguard;
pub mod system_info;
mod throughput;
mod top_10;
mod top_flows;
mod tree_capacity;
mod tree_summary;
mod tree_summary_l2;

use crate::system_stats::SystemStats;
pub use network_tree::all_circuits;

/// Runs a periodic tick to feed data to the node manager.
pub(super) async fn channel_ticker(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
) {
    debug!("Starting channel tickers");
    one_second_cadence(channels.clone(), bus_tx.clone(), system_usage_tx.clone()).await;
}

async fn one_second_cadence(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
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
            top_10::top_10_uploaders(channels.clone(), bus_tx.clone()),
            top_10::worst_10_downloaders(channels.clone(), bus_tx.clone()),
            top_10::worst_10_retransmit(channels.clone(), bus_tx.clone()),
            top_flows::top_flows_bytes(channels.clone(), bus_tx.clone()),
            top_flows::top_flows_rate(channels.clone(), bus_tx.clone()),
            asn_top::asn_top(channels.clone(), bus_tx.clone()),
            flow_endpoints::endpoints_by_country(channels.clone(), bus_tx.clone()),
            flow_endpoints::ether_protocols(channels.clone(), bus_tx.clone()),
            flow_endpoints::ip_protocols(channels.clone(), bus_tx.clone()),
            flow_endpoints::flow_duration(channels.clone(), bus_tx.clone()),
            endpoint_latlon::endpoint_latlon(channels.clone(), bus_tx.clone()),
            tree_summary::tree_summary(channels.clone(), bus_tx.clone()),
            tree_summary_l2::tree_summary_l2(channels.clone()),
            network_tree::all_subscribers(channels.clone(), bus_tx.clone()),
            queue_stats_total::queue_stats_totals(channels.clone()),
            network_tree::network_tree(channels.clone(), bus_tx.clone()),
            circuit_capacity::circuit_capacity(channels.clone()),
            tree_capacity::tree_capacity(channels.clone()),
            system_info::cpu_info(channels.clone(), system_usage_tx.clone()),
            system_info::ram_info(channels.clone(), system_usage_tx.clone()),
            retransmits::tcp_retransmits(channels.clone()),
            stormguard::stormguard_ticker(channels.clone(), bus_tx.clone()),
            bakery::bakery_ticker(channels.clone(), bus_tx.clone()),
            executive_heatmaps::executive_heatmaps(channels.clone(), bus_tx.clone()),
        );

        channels.clean().await;
    }
}
