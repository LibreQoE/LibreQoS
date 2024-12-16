use crate::lts2_sys::shared_types::{CircuitRtt, IngestSession};

pub(crate) fn add_circuit_rtt(message: &mut IngestSession, queue: &mut Vec<CircuitRtt>) {
    while let Some(msg) = queue.pop() {
        message.circuit_rtt.push(CircuitRtt {
            timestamp: msg.timestamp,
            circuit_hash: msg.circuit_hash,
            median_rtt: msg.median_rtt,
        });
    }
}
