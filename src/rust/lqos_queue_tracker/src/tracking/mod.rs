use std::sync::atomic::AtomicBool;

use crate::{
    circuit_to_queue::CIRCUIT_TO_QUEUE, interval::QUEUE_MONITOR_INTERVAL, queue_store::QueueStore,
    tracking::reader::read_named_queue_from_interface,
};
use log::{info, warn, error};
use lqos_config::LibreQoSConfig;
use nix::sys::time::TimeSpec;
use nix::sys::time::TimeValLike;
use nix::sys::timerfd::ClockId;
use nix::sys::timerfd::Expiration;
use nix::sys::timerfd::TimerFd;
use nix::sys::timerfd::TimerFlags;
use nix::sys::timerfd::TimerSetTimeFlags;
use rayon::prelude::{IntoParallelRefMutIterator, ParallelIterator};
mod reader;
mod watched_queues;
use watched_queues::WATCHED_QUEUES;
pub use watched_queues::{add_watched_queue, still_watching};
use self::watched_queues::expire_watched_queues;

fn track_queues() {
    let mut watching = WATCHED_QUEUES.write();
    if watching.is_empty() {
        //info!("No queues marked for read.");
        return; // There's nothing to do - bail out fast
    }
    let config = LibreQoSConfig::load();
    if config.is_err() {
        warn!("Unable to read LibreQoS config. Skipping queue collection cycle.");
        return;
    }
    let config = config.unwrap();
    watching.par_iter_mut().for_each(|q| {
        let (circuit_id, download_class, upload_class) = q.get();

        let (download, upload) = if config.on_a_stick_mode {
            (
                read_named_queue_from_interface(&config.internet_interface, download_class),
                read_named_queue_from_interface(&config.internet_interface, upload_class),
            )
        } else {
            (
                read_named_queue_from_interface(&config.isp_interface, download_class),
                read_named_queue_from_interface(&config.internet_interface, download_class),
            )
        };

        if let Ok(download) = download {
            if let Ok(upload) = upload {
                let mut mapping = CIRCUIT_TO_QUEUE.write();
                if let Some(circuit) = mapping.get_mut(circuit_id) {
                    circuit.update(&download[0], &upload[0]);
                } else {
                    // It's new: insert it
                    mapping.insert(
                        circuit_id.to_string(),
                        QueueStore::new(download[0].clone(), upload[0].clone()),
                    );
                }
            }
        }
    });

    std::mem::drop(watching); // Release the lock
    expire_watched_queues();
}

pub fn spawn_queue_monitor() {
    std::thread::spawn(|| {
        // Setup the queue monitor loop
        info!("Starting Queue Monitor Thread.");
        let interval_ms = if let Ok(config) = lqos_config::EtcLqos::load() {
            config.queue_check_period_ms
        } else {
            1000
        };
        QUEUE_MONITOR_INTERVAL.store(
            interval_ms,
            std::sync::atomic::Ordering::Relaxed,
        );
        info!("Queue check period set to {interval_ms} ms.");

        // Setup the Linux timer fd system
        let monitor_busy = AtomicBool::new(false);
        if let Ok(timer) = TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::empty()) {
            if timer.set(Expiration::Interval(TimeSpec::milliseconds(interval_ms as i64)), TimerSetTimeFlags::TFD_TIMER_ABSTIME).is_ok() {
                loop {
                    if timer.wait().is_ok() {
                        if monitor_busy.load(std::sync::atomic::Ordering::Relaxed) {
                            warn!("Queue tick fired while another queue read is ongoing. Skipping this cycle.");
                        } else {
                            monitor_busy.store(true, std::sync::atomic::Ordering::Relaxed);
                            //info!("Queue tracking timer fired.");
                            track_queues();
                            monitor_busy.store(false, std::sync::atomic::Ordering::Relaxed);
                        }
                    } else {
                        error!("Error in timer wait (Linux fdtimer). This should never happen.");
                    }
                }
            } else {
                error!("Unable to set the Linux fdtimer timer interval. Queues will not be monitored.");
            }
        } else {
            error!("Unable to acquire Linux fdtimer. Queues will not be monitored.");
        }
    });
}
