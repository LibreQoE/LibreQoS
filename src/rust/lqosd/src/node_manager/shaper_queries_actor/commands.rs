use crate::node_manager::local_api::lts::{AsnFlowSizeWeb, FlowCountViewWeb, FullPacketData, PercentShapedWeb, RecentMedians, ShaperRttHistogramEntry, ThroughputData, Top10Circuit, Worst10RttCircuit, Worst10RxmitCircuit};

pub enum ShaperQueryCommand {
    ShaperThroughput { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<ThroughputData>> },
    ShaperPackets { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<FullPacketData>> },
    ShaperPercent { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<PercentShapedWeb>> },
    ShaperFlows { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<FlowCountViewWeb>> },
    ShaperRttHistogram { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<ShaperRttHistogramEntry>> },
    ShaperTopDownloaders { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<Top10Circuit>> },
    ShaperWorstRtt { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<Worst10RttCircuit>> },
    ShaperWorstRxmit { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<Worst10RxmitCircuit>> },
    ShaperTopFlows { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<AsnFlowSizeWeb>> },
    ShaperRecentMedian { reply: tokio::sync::oneshot::Sender<Vec<RecentMedians>> },
}