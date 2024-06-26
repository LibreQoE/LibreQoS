#[derive(PartialEq, Clone, Copy, Debug)]
pub enum PublishedChannels {
    /// Provides a 1-second tick notification to the client
    Cadence,
    Throughput,
    RttHistogram,
    FlowCount,
}

impl PublishedChannels {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            PublishedChannels::Throughput => "throughput",
            PublishedChannels::RttHistogram => "rttHistogram",
            PublishedChannels::FlowCount => "flowCount",
            PublishedChannels::Cadence => "cadence",
        }
    }

    pub(super) fn from_str(s: &str) -> Option<Self> {
        match s {
            "throughput" => Some(Self::Throughput),
            "rttHistogram" => Some(Self::RttHistogram),
            "flowCount" => Some(Self::FlowCount),
            "cadence" => Some(Self::Cadence),
            _ => None,
        }
    }
}