use crate::{circuit_to_queue::CIRCUIT_TO_QUEUE, still_watching};
use lqos_bus::BusResponse;

pub fn get_raw_circuit_data(circuit_id: &str) -> BusResponse {
  still_watching(circuit_id);
  if let Some(circuit) = CIRCUIT_TO_QUEUE.get(circuit_id) {
    if let Ok(json) = serde_json::to_string(circuit.value()) {
      BusResponse::RawQueueData(json)
    } else {
      BusResponse::RawQueueData(String::new())
    }
  } else {
    BusResponse::RawQueueData(String::new())
  }
}
