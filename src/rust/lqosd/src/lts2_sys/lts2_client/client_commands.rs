use std::sync::{mpsc, OnceLock};
use anyhow::{bail, Result};
use tokio::sync::oneshot;
use crate::lts2_sys::lts2_client::ingestor;
use crate::lts2_sys::shared_types::FreeTrialDetails;

/// Commands that can be sent to the LTS client.
pub(crate) enum LtsClientCommand {
    /// Request the creation of a free trial. The response is returned to the sender, either a license key of `FAIL`.
    RequestFreeTrial(FreeTrialDetails, oneshot::Sender<String>),
    /// Pass timeseries data to the ingestor for submission to the LTS server.
    IngestData(ingestor::commands::IngestorCommand),
    IngestBatchComplete,
    /// Request the current license status. The response is returned to the sender. It matches the
    /// enum values in `LtsStatus`.
    LicenseStatus(oneshot::Sender<i32>),
    /// Request the number of days remaining in the free trial. The response is returned to the sender.
    TrialDaysRemaining(oneshot::Sender<i32>),
}

/// Holds the LTS command channel. C API calls inject commands into this channel,
/// which is processed asynchronously in the core.
static CHANNEL_HOLDER: OnceLock<mpsc::Sender<LtsClientCommand>> = OnceLock::new();

/// Set the command channel for the LTS client.
/// This should only be called once.
pub(crate) fn set_command_channel(sender: mpsc::Sender<LtsClientCommand>) -> Result<()> {
    if CHANNEL_HOLDER.set(sender).is_ok() {
        Ok(())
    } else {
        bail!("LTS command channel already set")
    }
}

/// Get the command channel for the LTS client.
/// This should only be called after the channel has been set.
pub(crate) fn get_command_channel() -> Result<mpsc::Sender<LtsClientCommand>> {
    if let Some(sender) = CHANNEL_HOLDER.get() {
        Ok(sender.clone())
    } else {
        bail!("LTS command channel not set")
    }
}