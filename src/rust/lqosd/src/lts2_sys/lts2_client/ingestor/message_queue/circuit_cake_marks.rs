use crate::lts2_sys::shared_types::{CircuitCakeMarks, IngestSession};

pub(crate) fn add_circuit_cake_marks(message: &mut IngestSession, queue: &mut Vec<CircuitCakeMarks>) {
    while let Some(circuit) = queue.pop() {
        message.circuit_cake_marks.push(CircuitCakeMarks {
            timestamp: circuit.timestamp,
            circuit_hash: circuit.circuit_hash,
            cake_marks_down: circuit.cake_marks_down,
            cake_marks_up: circuit.cake_marks_up,
        });
    }
}
