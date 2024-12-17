use crate::lts2_sys::shared_types::{CircuitCakeDrops, IngestSession};

pub(crate) fn add_circuit_cake_drops(message: &mut IngestSession, queue: &mut Vec<CircuitCakeDrops>) {
    while let Some(msg) = queue.pop() {
        message.circuit_cake_drops.push(msg);
    }
}
