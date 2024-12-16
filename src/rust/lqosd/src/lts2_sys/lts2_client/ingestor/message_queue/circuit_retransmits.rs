use crate::lts2_sys::shared_types::{CircuitRetransmits, IngestSession};

pub(crate) fn add_circuit_retransmits(message: &mut IngestSession, queue: &mut Vec<CircuitRetransmits>) {
    while let Some(msg) = queue.pop() {
        message.circuit_retransmits.push(CircuitRetransmits {
            timestamp: msg.timestamp,
            circuit_hash: msg.circuit_hash,
            tcp_retransmits_down: msg.tcp_retransmits_down,
            tcp_retransmits_up: msg.tcp_retransmits_up,
        });
    }
}
