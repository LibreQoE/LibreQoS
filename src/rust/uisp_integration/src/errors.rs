use thiserror::Error;

/// Error types for UISP Integration
#[derive(Error, Debug, PartialEq)]
pub enum UispIntegrationError {
    #[error("Unable to load configuration")]
    CannotLoadConfig,
    #[error("UISP Integration is Disabled")]
    IntegrationDisabled,
    #[error("Unknown Integration Strategy")]
    UnknownIntegrationStrategy,
    #[error("Error contacting UISP")]
    UispConnectError,
    #[error("Root site not found: {0}")]
    NoRootSite(String),
    #[error("Unknown Site Type")]
    UnknownSiteType,
    #[error("CSV Error")]
    CsvError,
    #[error("Unable to write network.json")]
    WriteNetJson,
    #[error("Unable to write circuit_anchors.json")]
    WriteCircuitAnchors,
    #[error("Bad IP")]
    BadIp,
    #[error("Bad IP range: {0}")]
    BadIpRange(String),
}
