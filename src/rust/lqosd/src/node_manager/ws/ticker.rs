use std::sync::Arc;

use tokio::{join, spawn};
use tokio::sync::mpsc::Sender;
use tracing::info;
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

pub use network_tree::{Circuit, all_circuits};

/// Runs a periodic tick to feed data to the node manager.
pub(super) async fn channel_ticker(channels: Arc<PubSub>, bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>) {
    info!("Starting channel tickers");
    join!(
        one_second_cadence(channels.clone(), bus_tx.clone()),
        two_second_cadence(channels.clone(), bus_tx.clone()),
        five_second_cadence(channels.clone(), bus_tx.clone())
    );
}

async fn one_second_cadence(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second

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
        );

        //let mc = channels.clone(); spawn(async move { cadence::cadence(mc).await });
        //let mc = channels.clone(); spawn(async move { throughput::throughput(mc).await });
        //let mc = channels.clone(); spawn(async move { rtt_histogram::rtt_histo(mc).await });
        //let mc = channels.clone(); spawn(async move { flow_counter::flow_count(mc).await });
        //let mc = channels.clone(); spawn(async move { top_10::top_10_downloaders(mc).await });
        //let mc = channels.clone(); spawn(async move { top_10::worst_10_downloaders(mc).await });
        //let mc = channels.clone(); spawn(async move { top_10::worst_10_retransmit(mc).await });
        //let mc = channels.clone(); spawn(async move { top_flows::top_flows_bytes(mc).await });
        //let mc = channels.clone(); spawn(async move { top_flows::top_flows_rate(mc).await });
        //let mc = channels.clone(); spawn(async move { flow_endpoints::endpoints_by_country(mc).await });
        //let mc = channels.clone(); spawn(async move { flow_endpoints::ether_protocols(mc).await });
        //let mc = channels.clone(); spawn(async move { flow_endpoints::ip_protocols(mc).await });
        //let mc = channels.clone(); spawn(async move { flow_endpoints::flow_duration(mc).await });
        //let mc = channels.clone(); spawn(async move { tree_summary::tree_summary(mc).await });
        //let mc = channels.clone(); spawn(async move { network_tree::network_tree(mc).await });
        //let mc = channels.clone(); spawn(async move { circuit_capacity::circuit_capacity(mc).await });
        //let mc = channels.clone(); spawn(async move { tree_capacity::tree_capacity(mc).await });

        channels.clean().await;
    }
}

async fn two_second_cadence(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second

        join!(
            queue_stats_total::queue_stats_totals(channels.clone()),
            network_tree::all_subscribers(channels.clone()),
        );
    }
}

async fn five_second_cadence(
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await; // Once per second

        join!(
            system_info::cpu_info(channels.clone()),
            system_info::ram_info(channels.clone()),
        );
    }
}