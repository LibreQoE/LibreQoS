use std::time::Instant;
use crate::{
  circuit_to_queue::CIRCUIT_TO_QUEUE, interval::QUEUE_MONITOR_INTERVAL,
  queue_store::QueueStore, tracking::reader::read_named_queue_from_interface,
};
use log::info;
use lqos_utils::fdtimer::periodic;
mod reader;
mod watched_queues;
mod all_queue_data;
pub use all_queue_data::*;

use self::watched_queues::expire_watched_queues;
use watched_queues::WATCHED_QUEUES;
pub use watched_queues::{add_watched_queue, still_watching};
use crate::queue_structure::{QUEUE_STRUCTURE, QueueNode};
use crate::queue_types::QueueType;
use crate::tracking::reader::read_all_queues_from_interface;

fn track_queues() {
  if WATCHED_QUEUES.is_empty() {
    //info!("No queues marked for read.");
    return; // There's nothing to do - bail out fast
  }
  let config = lqos_config::load_config();
  if config.is_err() {
    //warn!("Unable to read LibreQoS config. Skipping queue collection cycle.");
    return;
  }
  let config = config.unwrap();
  WATCHED_QUEUES.iter_mut().for_each(|q| {
    let (circuit_id, download_class, upload_class) = q.get();

    let (download, upload) = if config.on_a_stick_mode() {
      (
        read_named_queue_from_interface(
          &config.internet_interface(),
          download_class,
        ),
        read_named_queue_from_interface(
          &config.internet_interface(),
          upload_class,
        ),
      )
    } else {
      (
        read_named_queue_from_interface(&config.isp_interface(), download_class),
        read_named_queue_from_interface(
          &config.internet_interface(),
          download_class,
        ),
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
            info!(
              "No queue data returned for {}, {}/{} found.",
              circuit_id.to_string(),
              download.len(),
              upload.len()
            );
            info!("You probably want to run LibreQoS.py");
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

fn connect_queues_to_circuit(structure: &[QueueNode], queues: &[QueueType]) -> Vec<TrackedQueue> {
  queues
      .iter()
      .filter_map(|q| {
        if let QueueType::Cake(cake) = q {
          let (major, minor) = cake.parent.get_major_minor();
          if let Some (s) = structure.iter().find(|s| s.class_major == major as u32 && s.class_minor == minor as u32) {
            if let Some(circuit_hash) = &s.circuit_hash {
              let marks: u32 = cake.tins.iter().map(|tin| tin.ecn_marks).sum();
              if cake.drops > 0 || marks > 0 {
                return Some(TrackedQueue {
                  circuit_hash: *circuit_hash,
                  drops: cake.drops as u64,
                  marks: marks as u64,
                })
              }
            }
          }
        }
        None
      })
      .collect()
}

fn connect_queues_to_circuit_up(structure: &[QueueNode], queues: &[QueueType]) -> Vec<TrackedQueue> {
  queues
      .iter()
      .filter_map(|q| {
        if let QueueType::Cake(cake) = q {
          let (major, minor) = cake.parent.get_major_minor();
          if let Some (s) = structure.iter().find(|s| s.up_class_major == major as u32 && s.class_minor == minor as u32) {
            if let Some(circuit_hash) = &s.circuit_hash {
              let marks: u32 = cake.tins.iter().map(|tin| tin.ecn_marks).sum();
              if cake.drops > 0 || marks > 0 {
                return Some(TrackedQueue {
                  circuit_hash: *circuit_hash,
                  drops: cake.drops as u64,
                  marks: marks as u64,
                })
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
  let structure = QUEUE_STRUCTURE.read().unwrap();
  if let Some(structure) = &structure.maybe_queues {
    if let Ok(config) = lqos_config::load_config() {
      // Get all the queues
      let (download, upload) = if config.on_a_stick_mode() {
        let all_queues = read_all_queues_from_interface(&config.internet_interface());
        let (download, upload) = if let Ok(q) = all_queues {
          let download = connect_queues_to_circuit(&structure, &q);
          let upload = connect_queues_to_circuit_up(&structure, &q);
          (download, upload)
        } else {
          (Vec::new(), Vec::new())
        };
        (download, upload)
      } else {
        let all_queues_down = read_all_queues_from_interface(&config.internet_interface());
        let all_queues_up = read_all_queues_from_interface(&config.isp_interface());

        let download = if let Ok(q) = all_queues_down {
          connect_queues_to_circuit(&structure, &q)
        } else {
          Vec::new()
        };
        let upload = if let Ok(q) = all_queues_up {
          connect_queues_to_circuit(&structure, &q)
        } else {
          Vec::new()
        };
        (download, upload)
      };

      //println!("{}", download.len() + upload.len());
      ALL_QUEUE_SUMMARY.ingest_batch(download, upload);
    } else {
      log::warn!("(TC monitor) Unable to read configuration");
    }
  } else {
    log::warn!("(TC monitor) Not reading queues due to structure not yet ready");
  }
  let elapsed = start.elapsed();
  log::debug!("(TC monitor) Completed in {:.5} seconds", elapsed.as_secs_f32());
}

/// Spawns a thread that periodically reads the queue statistics from
/// the Linux `tc` shaper, and stores them in a `QueueStore` for later
/// retrieval.
pub fn spawn_queue_monitor() {
  std::thread::spawn(|| {
    // Setup the queue monitor loop
    info!("Starting Queue Monitor Thread.");
    let interval_ms = if let Ok(config) = lqos_config::load_config() {
      config.queue_check_period_ms
    } else {
      1000
    };
    QUEUE_MONITOR_INTERVAL
      .store(interval_ms, std::sync::atomic::Ordering::Relaxed);
    info!("Queue check period set to {interval_ms} ms.");

    // Setup the Linux timer fd system
    periodic(interval_ms, "Queue Reader", &mut || {
      track_queues();
    });
  });

  // Set up a 2nd thread to periodically gather ALL the queue stats
  std::thread::spawn(|| {
    periodic(2000, "All Queues", &mut || {
      all_queue_reader();
    })
  });
}
