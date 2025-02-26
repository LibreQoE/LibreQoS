use crate::lts2_sys::shared_types::{CircuitThroughput, IngestSession};

pub(crate) fn add_circuit_throughput(
    message: &mut IngestSession,
    queue: &mut Vec<CircuitThroughput>,
) {
    while let Some(msg) = queue.pop() {
        message.circuit_throughput.push(msg);
    }
}
