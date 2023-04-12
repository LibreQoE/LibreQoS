use crate::{
  circuit_to_queue::CIRCUIT_TO_QUEUE, queue_store::QueueStore, still_watching,
};
use lqos_bus::BusResponse;

/// Retrieves the raw queue data for a given circuit ID.
/// 
/// # Arguments
/// * `circuit_id` - The circuit ID to retrieve data for.
pub fn get_raw_circuit_data(circuit_id: &str) -> BusResponse {
  still_watching(circuit_id);
  if let Some(circuit) = CIRCUIT_TO_QUEUE.get(circuit_id) {
    let cv: QueueStore = circuit.value().clone();
    let transit = Box::new(cv.into());
    BusResponse::RawQueueData(Some(transit))
  } else {
    BusResponse::RawQueueData(None)
  }
}
