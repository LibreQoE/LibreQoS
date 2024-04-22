use thiserror::Error;

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
    #[error("Root site not found")]
    NoRootSite,
    #[error("Unknown Site Type")]
    UnknownSiteType,
    #[error("CSV Error")]
    CsvError,
}
