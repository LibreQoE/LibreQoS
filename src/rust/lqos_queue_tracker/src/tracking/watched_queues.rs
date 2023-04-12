use crate::queue_structure::QUEUE_STRUCTURE;
use dashmap::DashMap;
use log::{info, warn};
use lqos_bus::TcHandle;
use lqos_sys::num_possible_cpus;
use lqos_utils::unix_time::unix_now;
use once_cell::sync::Lazy;

pub(crate) static WATCHED_QUEUES: Lazy<DashMap<String, WatchedQueue>> =
  Lazy::new(DashMap::new);

#[derive(PartialEq, Eq, Hash)]
pub(crate) struct WatchedQueue {
  circuit_id: String,
  expires_unix_time: u64,
  download_class: TcHandle,
  upload_class: TcHandle,
}

impl WatchedQueue {
  pub(crate) fn get(&self) -> (&str, TcHandle, TcHandle) {
    (&self.circuit_id, self.download_class, self.upload_class)
  }

  pub(crate) fn refresh_timer(&mut self) {
    self.expires_unix_time = expiration_in_the_future();
  }
}

pub fn expiration_in_the_future() -> u64 {
  unix_now().unwrap_or(0) + 10
}

/// Start watching a queue. This will cause the queue to be read
/// periodically, and its statistics stored in the `QueueStore`.
/// If the queue is already being watched, this function will
/// do nothing.
/// 
/// # Arguments
/// * `circuit_id` - The circuit ID to watch
pub fn add_watched_queue(circuit_id: &str) {
  //info!("Watching queue {circuit_id}");
  let max = num_possible_cpus().unwrap() * 2;
  {
    if WATCHED_QUEUES.contains_key(circuit_id) {
      warn!("Queue {circuit_id} is already being watched. Duplicate ignored.");
      return; // No duplicates, please
    }

    if WATCHED_QUEUES.len() > max as usize {
      warn!(
        "Watching too many queues - didn't add {circuit_id} to watch list."
      );
      return; // Too many watched pots
    }
  }

  if let Some(queues) = &QUEUE_STRUCTURE.read().unwrap().maybe_queues {
    if let Some(circuit) = queues.iter().find(|c| {
      c.circuit_id.is_some() && c.circuit_id.as_ref().unwrap() == circuit_id
    }) {
      let new_watch = WatchedQueue {
        circuit_id: circuit.circuit_id.as_ref().unwrap().clone(),
        expires_unix_time: expiration_in_the_future(),
        download_class: circuit.class_id,
        upload_class: circuit.up_class_id,
      };

      WATCHED_QUEUES.insert(circuit.circuit_id.as_ref().unwrap().clone(), new_watch);
      //info!("Added {circuit_id} to watched queues. Now watching {} queues.", WATCHED_QUEUES.len());
    } else {
      warn!("No circuit ID of {circuit_id}");
    }
  } else {
    warn!("Unable to access watched queue list. Try again later.");
  }
}

pub(crate) fn expire_watched_queues() {
  let now = unix_now().unwrap_or(0);
  WATCHED_QUEUES.retain(|_,w| w.expires_unix_time > now);
}

/// Indicates that a watched queue is still being watched. Update the
/// expiration time for the queue.
/// 
/// # Arguments
/// * `circuit_id` - The circuit ID to watch
pub fn still_watching(circuit_id: &str) {
  if let Some(mut q) = WATCHED_QUEUES.get_mut(circuit_id) {
    //info!("Still watching circuit: {circuit_id}");
    q.refresh_timer();
  } else {
    info!("Still watching circuit, but it had expired: {circuit_id}");
    add_watched_queue(circuit_id);
  }
}
