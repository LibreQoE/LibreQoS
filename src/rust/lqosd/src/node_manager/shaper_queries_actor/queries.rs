use crate::lts2_sys::control_channel::{
    ControlChannelCommand, HistoryQueryResultPayload, RemoteInsightRequest,
};
use crate::node_manager::local_api::lts::{
    AsnFlowSizeWeb, CakeData, FlowCountViewWeb, FullPacketData, PercentShapedWeb, RecentMedians,
    ShaperRttHistogramEntry, ThroughputData, Top10Circuit, Worst10RttCircuit, Worst10RxmitCircuit,
};
use crate::node_manager::shaper_queries_actor::ShaperQueryCommand;
use crate::node_manager::shaper_queries_actor::caches::Caches;
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::{info, warn};

#[derive(Clone)]
struct HistoryClient {
    control_tx: tokio::sync::mpsc::Sender<ControlChannelCommand>,
}

impl HistoryClient {
    fn new(control_tx: tokio::sync::mpsc::Sender<ControlChannelCommand>) -> Self {
        Self { control_tx }
    }

    async fn request(
        &self,
        request: RemoteInsightRequest,
    ) -> Result<HistoryQueryResultPayload, ()> {
        let (tx, rx) = oneshot::channel();
        if self
            .control_tx
            .send(ControlChannelCommand::FetchHistory {
                request,
                responder: tx,
            })
            .await
            .is_err()
        {
            return Err(());
        }

        match rx.await {
            Ok(result) => result,
            Err(_) => Err(()),
        }
    }
}

macro_rules! shaper_query {
    ($type:ty, $caches:expr, $seconds:expr, $reply:expr, $client:expr, $request:expr) => {
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
        let my_client = $client.clone();
        let my_reply = $reply;
        let request_value = $request;
        let seconds_value = $seconds;
        info!("SQ Spawning");
        tokio::spawn(async move {
            let request = request_value;
            let mut attempts = 0;
            loop {
                info!("SQ Requesting history (attempt {})", attempts + 1);
                match my_client.request(request.clone()).await {
                    Ok(payload) => {
                        let data = payload.data.unwrap_or_default();
                        my_caches.store(payload.tag, payload.seconds, data).await;
                        if let Some(result) = my_caches.get::<$type>(seconds_value).await {
                            if my_reply.send(result).is_err() {
                                warn!("Failed to send. Oneshot died?");
                            }
                        } else {
                            if my_reply.send(Vec::<$type>::new()).is_err() {
                                warn!("Failed to send. Oneshot died?");
                            }
                        }
                        break;
                    }
                    Err(()) => {
                        attempts += 1;
                        if attempts >= 3 {
                            warn!("History request exhausted retries");
                            if my_reply.send(Vec::<$type>::new()).is_err() {
                                warn!("Failed to send. Oneshot died?");
                            }
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        });
        info!("SQ Returning");
    };
}

pub async fn shaper_queries(
    mut rx: tokio::sync::mpsc::Receiver<ShaperQueryCommand>,
    control_tx: tokio::sync::mpsc::Sender<ControlChannelCommand>,
) {
    info!("Starting the shaper query actor.");

    // Initialize the cache system
    let (caches, _broadcast_rx) = Caches::new();
    let history_client = HistoryClient::new(control_tx);

    while let Some(command) = rx.recv().await {
        info!("SQ: Received a command.");
        caches.cleanup().await;
        info!("SQ: Cleaned up the caches.");

        let client = history_client.clone();
        match command {
            ShaperQueryCommand::ShaperThroughput { seconds, reply } => {
                let request = RemoteInsightRequest::ShaperThroughput { seconds };
                shaper_query!(ThroughputData, caches, seconds, reply, client, request);
            }
            ShaperQueryCommand::ShaperPackets { seconds, reply } => {
                let request = RemoteInsightRequest::ShaperPackets { seconds };
                shaper_query!(FullPacketData, caches, seconds, reply, client, request);
            }
            ShaperQueryCommand::ShaperPercent { seconds, reply } => {
                let request = RemoteInsightRequest::ShaperPercent { seconds };
                shaper_query!(PercentShapedWeb, caches, seconds, reply, client, request);
            }
            ShaperQueryCommand::ShaperFlows { seconds, reply } => {
                let request = RemoteInsightRequest::ShaperFlows { seconds };
                shaper_query!(FlowCountViewWeb, caches, seconds, reply, client, request);
            }
            ShaperQueryCommand::ShaperRttHistogram { seconds, reply } => {
                let request = RemoteInsightRequest::ShaperRttHistogram { seconds };
                shaper_query!(
                    ShaperRttHistogramEntry,
                    caches,
                    seconds,
                    reply,
                    client,
                    request
                );
            }
            ShaperQueryCommand::ShaperTopDownloaders { seconds, reply } => {
                let request = RemoteInsightRequest::ShaperTopDownloaders { seconds };
                shaper_query!(Top10Circuit, caches, seconds, reply, client, request);
            }
            ShaperQueryCommand::ShaperWorstRtt { seconds, reply } => {
                let request = RemoteInsightRequest::ShaperWorstRtt { seconds };
                shaper_query!(Worst10RttCircuit, caches, seconds, reply, client, request);
            }
            ShaperQueryCommand::ShaperWorstRxmit { seconds, reply } => {
                let request = RemoteInsightRequest::ShaperWorstRxmit { seconds };
                shaper_query!(Worst10RxmitCircuit, caches, seconds, reply, client, request);
            }
            ShaperQueryCommand::ShaperTopFlows { seconds, reply } => {
                let request = RemoteInsightRequest::ShaperTopFlows { seconds };
                shaper_query!(AsnFlowSizeWeb, caches, seconds, reply, client, request);
            }
            ShaperQueryCommand::ShaperRecentMedian { reply } => {
                let request = RemoteInsightRequest::ShaperRecentMedians;
                shaper_query!(RecentMedians, caches, 0, reply, client, request);
            }
            ShaperQueryCommand::CakeTotals { seconds, reply } => {
                let request = RemoteInsightRequest::CakeStatsTotals { seconds };
                shaper_query!(CakeData, caches, seconds, reply, client, request);
            }
        }
        info!("SQ Looping");
    }
    warn!("Shaper query actor closing.")
}
