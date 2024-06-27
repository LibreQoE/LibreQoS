use strum::{Display, EnumIter, EnumString};

#[derive(PartialEq, Clone, Copy, Debug, EnumIter, Display, EnumString)]
pub enum PublishedChannels {
    /// Provides a 1-second tick notification to the client
    Cadence,
    Throughput,
    RttHistogram,
    FlowCount,
    TopDownloads,
    WorstRTT,
    WorstRetransmits,
    TopFlowsBytes,
    TopFlowsRate,
    EndpointsByCountry,
    EtherProtocols,
    IpProtocols,
}
