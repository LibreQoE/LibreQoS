use crate::node_manager::local_api::lts::{
    AsnFlowSizeWeb, FlowCountViewWeb, FullPacketData, PercentShapedWeb, RecentMedians,
    ShaperRttHistogramEntry, ThroughputData, Top10Circuit, Worst10RttCircuit, Worst10RxmitCircuit,
};
use crate::node_manager::shaper_queries_actor::caches::Caches;
use crate::node_manager::shaper_queries_actor::{ShaperQueryCommand, remote_insight};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

macro_rules! shaper_query {
    ($type:ty, $caches:expr, $seconds:expr, $reply:expr, $broadcast:expr, $call:expr) => {
        info!(
            "SQ Shaper query {} for {} seconds.",
            stringify!($type),
            $seconds
        );

        // Check for cache hit
        if let Some(result) = $caches.get::<$type>($seconds).await {
            let r = $reply.send(result);
            if r.is_err() {
                warn!("Failed to send. Oneshot died?");
            }
            continue;
        }

        let my_caches = $caches.clone();
        let mut my_broadcast_rx = $broadcast.resubscribe();
        info!("SQ Spawning");
        tokio::spawn(async move {
            let mut failed = true;
            let mut fail_count = 0;
            while failed && fail_count < 3 {
                info!("Making the call");
                failed = $call.await;
                fail_count += 1;
            }

            let mut timer = tokio::time::interval(Duration::from_secs(30));
            let mut timer_count = 0;
            loop {
                tokio::select! {
                    _ = timer.tick() => {
                        // Timed out
                        timer_count += 1;
                        if timer_count > 1 {
                            let _ = $reply.send(vec![]);
                            break;
                        }
                    }
                    Ok(_msg) = my_broadcast_rx.recv() => {
                        // Cache updated
                        info!("SQ Cache update hit");
                        if let Some(result) = my_caches.get($seconds).await {
                            if let Err(_e) = $reply.send(result) {
                                warn!("Failed to send. Oneshot died?");
                            }
                            break;
                        }
                    }
                }
            }
        });
        info!("SQ Returning");
    };
}

pub async fn shaper_queries(mut rx: tokio::sync::mpsc::Receiver<ShaperQueryCommand>) {
    info!("Starting the shaper query actor.");

    // Initialize the cache system
    let (caches, broadcast_rx) = Caches::new();
    let remote_insight = Arc::new(remote_insight::RemoteInsight::new(caches.clone()));

    while let Some(command) = rx.recv().await {
        info!("SQ: Received a command.");
        caches.cleanup().await;
        info!("SQ: Cleaned up the caches.");

        let my_remote_insight = remote_insight.clone();
        match command {
            ShaperQueryCommand::ShaperThroughput { seconds, reply } => {
                shaper_query!(
                    ThroughputData,
                    caches,
                    seconds,
                    reply,
                    broadcast_rx,
                    my_remote_insight.command(
                        remote_insight::RemoteInsightCommand::ShaperThroughput { seconds }
                    )
                );
            }
            ShaperQueryCommand::ShaperPackets { seconds, reply } => {
                shaper_query!(
                    FullPacketData,
                    caches,
                    seconds,
                    reply,
                    broadcast_rx,
                    my_remote_insight
                        .command(remote_insight::RemoteInsightCommand::ShaperPackets { seconds })
                );
            }
            ShaperQueryCommand::ShaperPercent { seconds, reply } => {
                shaper_query!(
                    PercentShapedWeb,
                    caches,
                    seconds,
                    reply,
                    broadcast_rx,
                    my_remote_insight
                        .command(remote_insight::RemoteInsightCommand::ShaperPercent { seconds })
                );
            }
            ShaperQueryCommand::ShaperFlows { seconds, reply } => {
                shaper_query!(
                    FlowCountViewWeb,
                    caches,
                    seconds,
                    reply,
                    broadcast_rx,
                    my_remote_insight
                        .command(remote_insight::RemoteInsightCommand::ShaperFlows { seconds })
                );
            }
            ShaperQueryCommand::ShaperRttHistogram { seconds, reply } => {
                shaper_query!(
                    ShaperRttHistogramEntry,
                    caches,
                    seconds,
                    reply,
                    broadcast_rx,
                    my_remote_insight.command(
                        remote_insight::RemoteInsightCommand::ShaperRttHistogram { seconds }
                    )
                );
            }
            ShaperQueryCommand::ShaperTopDownloaders { seconds, reply } => {
                shaper_query!(
                    Top10Circuit,
                    caches,
                    seconds,
                    reply,
                    broadcast_rx,
                    my_remote_insight.command(
                        remote_insight::RemoteInsightCommand::ShaperTopDownloaders { seconds }
                    )
                );
            }
            ShaperQueryCommand::ShaperWorstRtt { seconds, reply } => {
                shaper_query!(
                    Worst10RttCircuit,
                    caches,
                    seconds,
                    reply,
                    broadcast_rx,
                    my_remote_insight
                        .command(remote_insight::RemoteInsightCommand::ShaperWorstRtt { seconds })
                );
            }
            ShaperQueryCommand::ShaperWorstRxmit { seconds, reply } => {
                shaper_query!(
                    Worst10RxmitCircuit,
                    caches,
                    seconds,
                    reply,
                    broadcast_rx,
                    my_remote_insight.command(
                        remote_insight::RemoteInsightCommand::ShaperWorstRxmit { seconds }
                    )
                );
            }
            ShaperQueryCommand::ShaperTopFlows { seconds, reply } => {
                shaper_query!(
                    AsnFlowSizeWeb,
                    caches,
                    seconds,
                    reply,
                    broadcast_rx,
                    my_remote_insight
                        .command(remote_insight::RemoteInsightCommand::ShaperTopFlows { seconds })
                );
            }
            ShaperQueryCommand::ShaperRecentMedian { reply } => {
                shaper_query!(
                    RecentMedians,
                    caches,
                    0,
                    reply,
                    broadcast_rx,
                    my_remote_insight
                        .command(remote_insight::RemoteInsightCommand::ShaperRecentMedians)
                );
            }
        }
        info!("SQ Looping");
    }
    warn!("Shaper query actor closing.")
}
