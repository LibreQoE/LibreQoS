use log::{error, warn};
use nix::sys::{
  time::{TimeSpec, TimeValLike},
  timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags},
};
use std::sync::atomic::AtomicBool;

/// `periodic` runs a function at a given interval.
/// 
/// ## Parameters
/// 
/// * `interval_ms`: the interval in milliseconds.
/// * `task_name`: the name of the task to run.
/// * `tick_function`: the function to run at the given interval.
pub fn periodic(
  interval_ms: u64,
  task_name: &str,
  tick_function: &mut dyn FnMut(),
) {
  let monitor_busy = AtomicBool::new(false);
  if let Ok(timer) =
    TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::empty())
  {
    if timer
      .set(
        Expiration::Interval(TimeSpec::milliseconds(interval_ms as i64)),
        TimerSetTimeFlags::TFD_TIMER_ABSTIME,
      )
      .is_ok()
    {
      loop {
        if timer.wait().is_ok() {
          if monitor_busy.load(std::sync::atomic::Ordering::Relaxed) {
            warn!("{task_name} tick fired while another queue read is ongoing. Skipping this cycle.");
          } else {
            monitor_busy.store(true, std::sync::atomic::Ordering::Relaxed);
            //info!("Queue tracking timer fired.");
            tick_function();
            monitor_busy.store(false, std::sync::atomic::Ordering::Relaxed);
          }
        } else {
          error!(
            "Error in timer wait (Linux fdtimer). This should never happen."
          );
        }
      }
    } else {
      error!("Unable to set the Linux fdtimer timer interval. Queues will not be monitored.");
    }
  } else {
    error!("Unable to acquire Linux fdtimer. Queues will not be monitored.");
  }
}
