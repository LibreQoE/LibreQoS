use allocative::Allocative;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use strum::{Display, EnumIter, EnumString};

#[derive(
    PartialEq,
    Clone,
    Copy,
    Debug,
    EnumIter,
    Display,
    EnumString,
    Hash,
    Eq,
    Allocative,
    Serialize,
)]
pub enum PublishedChannels {
    /// Provides a 1-second tick notification to the client
    Cadence,
    Throughput,
    Retransmits,
    RttHistogram,
    FlowCount,
    TopDownloads,
    TopUploads,
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
    TreeSummaryL2,
    QueueStatsTotal,
    NetworkTree,
    NetworkTreeLite,
    NetworkTreeClients,
    CircuitCapacity,
    TreeCapacity,
    StormguardStatus,
    StormguardDebug,
    StormguardRuntime,
    EndpointLatLon,
    AsnTopDownload,
    AsnTopUpload,
    ExecutiveDashboardSummary,
    /// Sentinel for a channel the server does not publish — e.g. a retired
    /// channel a stale client still subscribes to (the K3-retired
    /// `BakeryStatus`/`BakeryActivity` class).  Never sent by the server;
    /// deserialization maps unrecognized names here so the websocket decoder
    /// can SKIP them instead of failing the whole request and spamming the log.
    Unknown,
}

/// Lenient deserialization: an unknown channel name decodes to
/// [`PublishedChannels::Unknown`] rather than a hard error.  The request
/// decoder then skips the subscription.  Serialization stays exact (the server
/// never emits `Unknown`).
impl<'de> Deserialize<'de> for PublishedChannels {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        Ok(PublishedChannels::from_str(&name).unwrap_or(PublishedChannels::Unknown))
    }
}

