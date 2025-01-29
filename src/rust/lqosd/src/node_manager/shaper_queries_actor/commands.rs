use crate::node_manager::local_api::lts::{FlowCountViewWeb, FullPacketData, PercentShapedWeb, ShaperRttHistogramEntry, ThroughputData};

pub enum ShaperQueryCommand {
    ShaperThroughput { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<ThroughputData>> },
    ShaperPackets { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<FullPacketData>> },
    ShaperPercent { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<PercentShapedWeb>> },
    ShaperFlows { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<FlowCountViewWeb>> },
    ShaperRttHistogram { seconds: i32, reply: tokio::sync::oneshot::Sender<Vec<ShaperRttHistogramEntry>> },
}