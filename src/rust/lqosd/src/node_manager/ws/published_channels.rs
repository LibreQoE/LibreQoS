use allocative::Allocative;
use strum::{Display, EnumIter, EnumString};

#[derive(PartialEq, Clone, Copy, Debug, EnumIter, Display, EnumString, Hash, Eq, Allocative)]
pub enum PublishedChannels {
    /// Provides a 1-second tick notification to the client
    Cadence,
    Throughput,
    Retransmits,
    RttHistogram,
    FlowCount,
    TopDownloads,
    WorstRTT,
    WorstRetransmits,
    TopFlowsBytes,
    TopFlowsRate,
    EndpointsByCountry,
    FlowDurations,
    EtherProtocols,
    IpProtocols,
    Cpu,
    Ram,
    TreeSummary,
    QueueStatsTotal,
    NetworkTree,
    NetworkTreeClients,
    CircuitCapacity,
    TreeCapacity,
    StormguardStatus,
    BakeryStatus,
    EndpointLatLon,
    AsnTopDownload,
    AsnTopUpload,
}
