use crate::{
    circuit_to_queue::CIRCUIT_TO_QUEUE, interval::QUEUE_MONITOR_INTERVAL, queue_store::QueueStore,
    tracking::reader::read_named_queue_from_interface,
};
use lqos_utils::fdtimer::periodic;
use std::time::{Duration, Instant};
use timerfd::{SetTimeFlags, TimerFd, TimerState};
use tracing::{debug, error, warn};
mod all_queue_data;
mod reader;
mod watched_queues;
pub use all_queue_data::*;

use self::watched_queues::expire_watched_queues;
use crate::queue_structure::{QUEUE_STRUCTURE, QueueNode};
use crate::queue_types::QueueType;
use crate::tracking::reader::read_all_queues_from_interface;
use watched_queues::WATCHED_QUEUES;
pub use watched_queues::{add_watched_queue, still_watching};

fn track_queues() {
    if WATCHED_QUEUES.is_empty() {
        //info!("No queues marked for read.");
        return; // There's nothing to do - bail out fast
    }
    let Ok(config) = lqos_config::load_config() else {
        return;
    };
    WATCHED_QUEUES.iter_mut().for_each(|q| {
        let (circuit_id, download_class, upload_class) = q.get();

        let (download, upload) = if config.on_a_stick_mode() {
            (
                read_named_queue_from_interface(&config.internet_interface(), download_class),
                read_named_queue_from_interface(&config.internet_interface(), upload_class),
            )
        } else {
            (
                read_named_queue_from_interface(&config.isp_interface(), download_class),
                read_named_queue_from_interface(&config.internet_interface(), download_class),
            )
        };

        if let Ok(download) = download {
            if let Ok(upload) = upload {
                if let Some(mut circuit) = CIRCUIT_TO_QUEUE.get_mut(circuit_id) {
                    if !download.is_empty() && !upload.is_empty() {
                        circuit.update(&download[0], &upload[0]);
                    }
                } else {
                    // It's new: insert it
                    if !download.is_empty() && !upload.is_empty() {
                        CIRCUIT_TO_QUEUE.insert(
                            circuit_id.to_string(),
                            QueueStore::new(download[0].clone(), upload[0].clone()),
                        );
                    } else {
                        debug!(
                            "No queue data returned for {}, {}/{} found.",
                            circuit_id.to_string(),
                            download.len(),
                            upload.len()
                        );
                        debug!("You probably want to run LibreQoS.py");
                    }
                }
            }
        }
    });

    expire_watched_queues();
}

/// Holds the CAKE marks/drops for a given queue/circuit.
pub struct TrackedQueue {
    circuit_hash: i64,
    drops: u64,
    marks: u64,
}

fn count_queue_types(queues: &[QueueType]) -> QueueCounts {
    let mut counts = QueueCounts::default();
    for queue in queues.iter() {
        match queue {
            QueueType::Cake(_) => counts.cake += 1,
            QueueType::Htb(_) => counts.htb += 1,
            _ => {}
        }
    }
    counts
}

fn connect_queues_to_circuit(structure: &[QueueNode], queues: &[QueueType]) -> Vec<TrackedQueue> {
    queues
        .iter()
        .filter_map(|q| {
            if let QueueType::Cake(cake) = q {
                //println!("{}", cake.parent.as_tc_string());
                //let (major, minor) = cake.parent.get_major_minor();
                //println!("{major:?}, {minor:?}");
                if let Some(s) = structure
                    .iter()
                    //.find(|s| s.class_major == major as u32 && s.class_minor == minor as u32)
                    .find(|s| cake.parent.as_tc_string() == s.class_id.as_tc_string())
                {
                    //println!("It matched!");
                    if let Some(circuit_hash) = &s.circuit_hash {
                        //println!("Circuit hash: {:?}", circuit_hash);
                        let marks: u32 = cake.tins.iter().map(|tin| tin.ecn_marks).sum();
                        if cake.drops > 0 || marks > 0 {
                            return Some(TrackedQueue {
                                circuit_hash: *circuit_hash,
                                drops: cake.drops as u64,
                                marks: marks as u64,
                            });
                        }
                    }
                }
            }
            None
        })
        .collect()
}

fn connect_queues_to_circuit_up(
    structure: &[QueueNode],
    queues: &[QueueType],
) -> Vec<TrackedQueue> {
    queues
        .iter()
        .filter_map(|q| {
            if let QueueType::Cake(cake) = q {
                let (major, minor) = cake.parent.get_major_minor();
                if let Some(s) = structure
                    .iter()
                    .find(|s| s.up_class_major == major as u32 && s.class_minor == minor as u32)
                {
                    if let Some(circuit_hash) = &s.circuit_hash {
                        let marks: u32 = cake.tins.iter().map(|tin| tin.ecn_marks).sum();
                        if cake.drops > 0 || marks > 0 {
                            return Some(TrackedQueue {
                                circuit_hash: *circuit_hash,
                                drops: cake.drops as u64,
                                marks: marks as u64,
                            });
                        }
                    }
                }
            }
            None
        })
        .collect()
}

fn all_queue_reader() {
    let start = Instant::now();
    let structure = QUEUE_STRUCTURE.load();
    if let Some(structure) = &structure.maybe_queues {
        if let Ok(config) = lqos_config::load_config() {
            // Get all the queues
            let (download, upload, queue_counts) = if config.on_a_stick_mode() {
                let all_queues = read_all_queues_from_interface(&config.internet_interface());
                let (download, upload, counts) = if let Ok(q) = all_queues {
                    let download = connect_queues_to_circuit(&structure, &q);
                    let upload = connect_queues_to_circuit_up(&structure, &q);
                    (download, upload, count_queue_types(&q))
                } else {
                    (Vec::new(), Vec::new(), QueueCounts::default())
                };
                (download, upload, counts)
            } else {
                let all_queues_down = read_all_queues_from_interface(&config.internet_interface());
                let all_queues_up = read_all_queues_from_interface(&config.isp_interface());

                let download = if let Ok(q) = &all_queues_down {
                    connect_queues_to_circuit(&structure, q)
                } else {
                    Vec::new()
                };
                let upload = if let Ok(q) = &all_queues_up {
                    connect_queues_to_circuit(&structure, q)
                } else {
                    Vec::new()
                };
                let counts_down = all_queues_down
                    .as_ref()
                    .map(|queues| count_queue_types(queues))
                    .unwrap_or_default();
                let counts_up = all_queues_up
                    .as_ref()
                    .map(|queues| count_queue_types(queues))
                    .unwrap_or_default();
                let counts = QueueCounts {
                    cake: counts_down.cake + counts_up.cake,
                    htb: counts_down.htb + counts_up.htb,
                };
                (download, upload, counts)
            };

            //println!("{}", download.len() + upload.len());
            ALL_QUEUE_SUMMARY.ingest_batch(download, upload, queue_counts);
        } else {
            warn!("(TC monitor) Unable to read configuration");
        }
    } else {
        warn!("(TC monitor) Not reading queues due to structure not yet ready");
    }
    let elapsed = start.elapsed();
    debug!(
        "(TC monitor) Completed in {:.5} seconds",
        elapsed.as_secs_f32()
    );
}

/// Spawns a thread that periodically reads the queue statistics from
/// the Linux `tc` shaper, and stores them in a `QueueStore` for later
/// retrieval.
pub fn spawn_queue_monitor() -> anyhow::Result<()> {
    std::thread::Builder::new()
        .name("Queue Monitor".to_string())
        .spawn(|| {
            // Setup the queue monitor loop
            debug!("Starting Queue Monitor Thread.");
            let interval_ms = if let Ok(config) = lqos_config::load_config() {
                config.queue_check_period_ms
            } else {
                1000
            };
            QUEUE_MONITOR_INTERVAL.store(interval_ms, std::sync::atomic::Ordering::Relaxed);
            debug!("Queue check period set to {interval_ms} ms.");

            // Setup the Linux timer fd system
            periodic(interval_ms, "Queue Reader", &mut || {
                track_queues();
            });
        })?;

    // Set up a 2nd thread to periodically gather ALL the queue stats
    std::thread::Builder::new()
        .name("All Queue Monitor".to_string())
        .spawn(|| {
            let mut interval_seconds = 2;
            let Ok(mut tfd) = TimerFd::new() else {
                error!("Unable to start timer file descriptor. All queue monitor cannot run.");
                return;
            };
            assert_eq!(tfd.get_state(), TimerState::Disarmed);
            tfd.set_state(
                TimerState::Periodic {
                    current: Duration::new(2, 0),
                    interval: Duration::new(interval_seconds, 0),
                },
                SetTimeFlags::Default,
            );
            let _ = tfd.read(); // Initial pause

            loop {
                all_queue_reader();

                // Sleep until the next second
                let missed_ticks = tfd.read();
                if missed_ticks > 1 {
                    warn!("All Queue Reader: Missed {} ticks", missed_ticks - 1);
                    interval_seconds = 2 + (missed_ticks - 1) as u64;
                    tfd.set_state(
                        TimerState::Periodic {
                            current: Duration::new(interval_seconds, 0),
                            interval: Duration::new(interval_seconds, 0),
                        },
                        SetTimeFlags::Default,
                    );
                }
            }
        })?;

    Ok(())
}
