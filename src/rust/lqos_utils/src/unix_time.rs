use log::{error, warn};
use nix::{
  sys::time::TimeSpec,
  time::{clock_gettime, ClockId},
};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Retrieves the current time, in seconds since the UNIX epoch.
/// Otherwise known as "unix time".
///
/// It can fail if the clock isn't ready.
pub fn unix_now() -> Result<u64, TimeError> {
  match SystemTime::now().duration_since(UNIX_EPOCH) {
    Ok(t) => Ok(t.as_secs()),
    Err(e) => {
      error!("Error determining the time in UNIX land: {:?}", e);
      Err(TimeError::ClockNotReady)
    }
  }
}

/// Return the time since boot, from the Linux kernel.
/// Can fail if the clock isn't ready yet.
pub fn time_since_boot() -> Result<TimeSpec, TimeError> {
  match clock_gettime(ClockId::CLOCK_BOOTTIME) {
    Ok(t) => Ok(t),
    Err(e) => {
      warn!("Clock not ready: {:?}", e);
      Err(TimeError::ClockNotReady)
    }
  }
}

/// Error type for time functions.
#[derive(Error, Debug)]
pub enum TimeError {
  /// The clock isn't ready yet.
  #[error("Clock not ready")]
  ClockNotReady,
}
