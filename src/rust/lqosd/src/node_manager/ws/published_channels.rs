use strum::EnumIter;

#[derive(PartialEq, Clone, Copy, Debug, EnumIter)]
pub enum PublishedChannels {
    /// Provides a 1-second tick notification to the client
    Cadence,
    Throughput,
    RttHistogram,
    FlowCount,
    Top10Downloaders,
    Worst10Downloaders,
    TopFlowsBytes,
}

impl PublishedChannels {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Throughput => "throughput",
            Self::RttHistogram => "rttHistogram",
            Self::FlowCount => "flowCount",
            Self::Cadence => "cadence",
            Self::Top10Downloaders => "top10downloaders",
            Self::Worst10Downloaders => "worst10downloaders",
            Self::TopFlowsBytes => "topFlowsBytes",
        }
    }

    pub(super) fn from_str(s: &str) -> Option<Self> {
        match s {
            "throughput" => Some(Self::Throughput),
            "rttHistogram" => Some(Self::RttHistogram),
            "flowCount" => Some(Self::FlowCount),
            "cadence" => Some(Self::Cadence),
            "top10downloaders" => Some(Self::Top10Downloaders),
            "worst10downloaders" => Some(Self::Worst10Downloaders),
            "topFlowsBytes" => Some(Self::TopFlowsBytes),
            _ => None,
        }
    }
}