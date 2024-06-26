mod cadence;
mod throughput;
mod rtt_histogram;
mod flow_counter;

use std::sync::Arc;
use crate::node_manager::ws::publish_subscribe::PubSub;

/// Runs a periodic tick to feed data to the node manager.
pub(super) async fn channel_ticker(channels: Arc<PubSub>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    loop {
        interval.tick().await; // Once per second

        tokio::join!(
            cadence::cadence(channels.clone()),
            throughput::throughput(channels.clone()),
            rtt_histogram::rtt_histo(channels.clone()),
            flow_counter::flow_count(channels.clone()),
        );
    }
}