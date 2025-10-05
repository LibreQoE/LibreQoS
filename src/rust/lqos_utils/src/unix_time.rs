use nix::{
    sys::time::TimeSpec,
    time::{ClockId, clock_gettime},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tracing::{error, warn};

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

/// Convert a time in nanoseconds since boot to a UNIX timestamp.
pub fn boot_time_nanos_to_unix_now(start_time_nanos_since_boot: u64) -> Result<u64, TimeError> {
    let time_since_boot = time_since_boot()?;
    let since_boot = Duration::from(time_since_boot);
    let boot_time = unix_now()? - since_boot.as_secs();
    Ok(boot_time + Duration::from_nanos(start_time_nanos_since_boot).as_secs())
}

/// Error type for time functions.
#[derive(Error, Debug)]
pub enum TimeError {
    /// The clock isn't ready yet.
    #[error("Clock not ready")]
    ClockNotReady,
}
