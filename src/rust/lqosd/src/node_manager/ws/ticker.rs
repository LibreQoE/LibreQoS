use std::sync::Arc;

use crate::node_manager::ws::publish_subscribe::PubSub;
use futures_util::FutureExt;
use lqos_bus::BusRequest;
use std::panic::AssertUnwindSafe;
use tokio::join;
use tokio::sync::mpsc::Sender;
use tokio::time::{Duration, timeout};
use tracing::{debug, warn};
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

const ONE_SECOND_TICKER_TIMEOUT: Duration = Duration::from_millis(950);

async fn ticker_with_timeout<T>(
    name: &'static str,
    fut: impl std::future::Future<Output = T>,
) {
    let result = timeout(ONE_SECOND_TICKER_TIMEOUT, AssertUnwindSafe(fut).catch_unwind()).await;
    match result {
        Ok(Ok(_)) => {}
        Ok(Err(panic)) => warn!(
            ticker = name,
            panic = panic_payload_to_string(&panic),
            "Ticker panicked"
        ),
        Err(_) => warn!(ticker = name, "Ticker timed out"),
    }
}

fn panic_payload_to_string(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

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
            ticker_with_timeout("cadence", cadence::cadence(channels.clone())),
            ticker_with_timeout(
                "throughput",
                throughput::throughput(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "rtt_histogram",
                rtt_histogram::rtt_histo(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "flow_counter",
                flow_counter::flow_count(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "top_10_downloaders",
                top_10::top_10_downloaders(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "top_10_uploaders",
                top_10::top_10_uploaders(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "worst_10_downloaders",
                top_10::worst_10_downloaders(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "worst_10_retransmit",
                top_10::worst_10_retransmit(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "top_flows_bytes",
                top_flows::top_flows_bytes(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "top_flows_rate",
                top_flows::top_flows_rate(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout("asn_top", asn_top::asn_top(channels.clone(), bus_tx.clone())),
            ticker_with_timeout(
                "endpoints_by_country",
                flow_endpoints::endpoints_by_country(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "ether_protocols",
                flow_endpoints::ether_protocols(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "ip_protocols",
                flow_endpoints::ip_protocols(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "flow_duration",
                flow_endpoints::flow_duration(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "endpoint_latlon",
                endpoint_latlon::endpoint_latlon(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "tree_summary",
                tree_summary::tree_summary(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "tree_summary_l2",
                tree_summary_l2::tree_summary_l2(channels.clone())
            ),
            ticker_with_timeout(
                "all_subscribers",
                network_tree::all_subscribers(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "queue_stats_totals",
                queue_stats_total::queue_stats_totals(channels.clone())
            ),
            ticker_with_timeout(
                "network_tree",
                network_tree::network_tree(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "circuit_capacity",
                circuit_capacity::circuit_capacity(channels.clone())
            ),
            ticker_with_timeout("tree_capacity", tree_capacity::tree_capacity(channels.clone())),
            ticker_with_timeout(
                "cpu_info",
                system_info::cpu_info(channels.clone(), system_usage_tx.clone())
            ),
            ticker_with_timeout(
                "ram_info",
                system_info::ram_info(channels.clone(), system_usage_tx.clone())
            ),
            ticker_with_timeout(
                "tcp_retransmits",
                retransmits::tcp_retransmits(channels.clone())
            ),
            ticker_with_timeout(
                "stormguard",
                stormguard::stormguard_ticker(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "bakery",
                bakery::bakery_ticker(channels.clone(), bus_tx.clone())
            ),
            ticker_with_timeout(
                "executive_heatmaps",
                executive_heatmaps::executive_heatmaps(channels.clone(), bus_tx.clone())
            ),
        );

        channels.clean().await;
    }
}
